//! Keeping open tabs named after what Claude calls their session. A tab's
//! title is never written directly: each source is recorded on the tab, and
//! [`crate::title::resolve`] decides which one shows.

use super::*;

impl App {
    /// Re-read every open tab's scanned and overlaid titles. Called whenever
    /// one of those sources may have moved: a rescan, the metadata overlay, a
    /// rename, a pane learning which conversation it runs.
    pub(super) fn retitle_tabs(&mut self) {
        for index in 0..self.workspace.tabs.len() {
            let (named, described) = self.tab_recorded_titles(index);
            if let Some(tab) = self.workspace.tabs.get_mut(index) {
                tab.set_recorded_titles(named.as_deref(), described.as_deref());
            }
        }
    }

    /// The scanned and overlaid titles of the tab at `index`'s conversation,
    /// owned so the tab can be written while they are held.
    fn tab_recorded_titles(&self, index: usize) -> (Option<String>, Option<String>) {
        let Some(claude_id) = self.tab_claude_session_id(index) else {
            return (None, None);
        };
        let (named, described) = self.recorded_titles(claude_id, self.record_for(claude_id));
        (named.map(str::to_owned), described.map(str::to_owned))
    }

    /// Give the tab at `index` a local name, unless naming it is Claude's to
    /// do — see [`Self::tab_names_through_claude`].
    pub(super) fn rename_tab(&mut self, index: usize, title: &str) -> Vec<Effect> {
        if !self.tab_names_through_claude(index) {
            self.workspace.rename_tab(index, title);
        }
        Vec::new()
    }

    /// Whether renaming the tab at `index` is Claude's to do — see
    /// [`Self::tab_claude_namer`].
    #[must_use]
    pub fn tab_names_through_claude(&self, index: usize) -> bool {
        self.tab_claude_namer(index).is_some()
    }

    /// The pane a rename of the tab at `index` is asked of, with `/rename`:
    /// its first pane — the one a tab is named after — when that pane runs
    /// Claude (see [`LiveSession::runs_claude`]). Such a tab takes no local
    /// name.
    #[must_use]
    pub fn tab_claude_namer(&self, index: usize) -> Option<SessionId> {
        let first = self.workspace.tabs.get(index)?.first_session();
        self.sessions
            .get(&first)
            .is_some_and(LiveSession::runs_claude)
            .then_some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;
    use crate::browser::SessionRecord;
    use crate::claude_command::{ClaudeCommand, PromptInput};

    /// Open a Claude tab launched as `launch`, labelled `repo`.
    fn open_claude(app: &mut App, launch: ClaudeLaunch) -> SessionId {
        match app
            .apply(Event::LaunchSession(LaunchSpec {
                cwd: Some("/repo".into()),
                launch: Launch::Claude(launch),
                placement: Placement::Foreground,
                title: "repo".into(),
            }))
            .as_slice()
        {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    /// A scanned record for conversation `id`, carrying the given titles.
    fn scanned(id: &str, custom: Option<&str>, ai: Option<&str>) -> SessionRecord {
        let mut r = record(id, "/repo", "the first prompt");
        r.digest.custom_title = custom.map(str::to_owned);
        r.digest.ai_title = ai.map(str::to_owned);
        r
    }

    fn title(app: &App) -> &str {
        app.workspace.tabs[0].display_title()
    }

    #[test]
    fn a_rescan_retitles_an_open_tab_with_claudes_rename() {
        let mut app = App::new();
        open_claude(&mut app, ClaudeLaunch::Resume("abc".into()));
        app.apply(Event::ScanCompleted(vec![scanned("abc", None, None)]));
        assert_eq!(title(&app), "the first prompt");

        app.apply(Event::ScanCompleted(vec![scanned(
            "abc",
            Some("auth refactor"),
            None,
        )]));
        assert_eq!(title(&app), "auth refactor");
    }

    #[test]
    fn a_rescan_retitles_a_fresh_tab_launched_under_a_minted_id() {
        let mut app = App::new();
        open_claude(&mut app, ClaudeLaunch::Fresh(Some("minted".into())));
        assert_eq!(title(&app), "repo", "nothing scanned yet");

        app.apply(Event::ScanCompleted(vec![scanned(
            "minted",
            Some("auth refactor"),
            None,
        )]));
        assert_eq!(title(&app), "auth refactor");
    }

    #[test]
    fn claudes_rename_outranks_a_live_title_and_a_live_title_the_ai_title() {
        let mut app = App::new();
        let session = open_claude(&mut app, ClaudeLaunch::Resume("abc".into()));
        app.apply(Event::ScanCompleted(vec![scanned(
            "abc",
            None,
            Some("Fix the login"),
        )]));
        assert_eq!(title(&app), "Fix the login");

        app.apply(Event::SessionTitleChanged {
            session,
            title: "Reading files".into(),
        });
        assert_eq!(title(&app), "Reading files", "live outranks the AI title");

        app.apply(Event::ScanCompleted(vec![scanned(
            "abc",
            Some("auth refactor"),
            Some("Fix the login"),
        )]));
        assert_eq!(title(&app), "auth refactor", "a rename outranks live");

        app.apply(Event::SessionTitleChanged {
            session,
            title: "Writing tests".into(),
        });
        assert_eq!(
            title(&app),
            "auth refactor",
            "and a later live title still does not displace it"
        );
    }

    #[test]
    fn a_tab_the_scan_does_not_know_keeps_its_launch_label() {
        let mut app = App::new();
        open_claude(&mut app, ClaudeLaunch::Resume("abc".into()));
        app.apply(Event::ScanCompleted(vec![scanned(
            "other",
            Some("someone else"),
            None,
        )]));
        assert_eq!(title(&app), "repo");
    }

    #[test]
    fn a_shell_tabs_local_name_survives_a_rescan() {
        let mut app = App::new();
        launch(&mut app, "sh");
        app.apply(Event::RenameTab {
            index: 0,
            title: "build".into(),
        });
        app.apply(Event::ScanCompleted(vec![scanned("abc", Some("x"), None)]));
        assert_eq!(title(&app), "build");
    }

    #[test]
    fn a_claude_tab_takes_no_local_name() {
        let mut app = App::new();
        open_claude(&mut app, ClaudeLaunch::Resume("abc".into()));
        app.apply(Event::RenameTab {
            index: 0,
            title: "mine".into(),
        });
        assert_eq!(app.workspace.tabs[0].custom_title, None);
        assert_eq!(title(&app), "repo");
    }

    #[test]
    fn a_tab_is_renamed_through_claude_when_its_first_pane_runs_claude() {
        let mut app = App::new();
        launch(&mut app, "sh");
        open_claude(&mut app, ClaudeLaunch::Fresh(None));
        app.apply(Event::SplitFocused(crate::workspace::SplitDir::Vertical));
        assert!(!app.tab_names_through_claude(0), "a shell tab");
        assert!(
            app.tab_names_through_claude(1),
            "a Claude tab, even with a shell split beside it"
        );
        assert!(!app.tab_names_through_claude(9), "no such tab");
    }

    #[test]
    fn claudes_rename_outranks_a_name_kept_in_the_sidebar() {
        let mut app = App::new();
        open_claude(&mut app, ClaudeLaunch::Resume("abc".into()));
        app.apply(Event::RenameSession {
            session: "abc".into(),
            title: "local".into(),
        });
        assert_eq!(title(&app), "local", "until Claude names the session");

        app.apply(Event::ScanCompleted(vec![scanned(
            "abc",
            Some("from claude"),
            None,
        )]));
        assert_eq!(title(&app), "from claude");
        let record = app.record_for("abc").expect("scanned").clone();
        assert_eq!(app.session_title(&record), "from claude", "and the sidebar");
    }

    #[test]
    fn a_sidebar_name_given_over_claudes_stands_until_claude_renames_again() {
        // The later naming wins: a local name given while Claude already had
        // one is a deliberate override (two sessions /clear left with the same
        // title), and a /rename typed after it is too.
        let mut app = App::new();
        open_claude(&mut app, ClaudeLaunch::Resume("abc".into()));
        app.apply(Event::ScanCompleted(vec![scanned(
            "abc",
            Some("shared"),
            None,
        )]));
        app.apply(Event::RenameSession {
            session: "abc".into(),
            title: "mine".into(),
        });
        assert_eq!(title(&app), "mine");
        app.apply(Event::ScanCompleted(vec![scanned(
            "abc",
            Some("shared"),
            None,
        )]));
        assert_eq!(
            title(&app),
            "mine",
            "a rescan of the same name changes nothing"
        );

        app.apply(Event::ScanCompleted(vec![scanned(
            "abc",
            Some("renamed"),
            None,
        )]));
        assert_eq!(title(&app), "renamed");
    }

    #[test]
    fn typing_a_rename_into_claude_leaves_the_local_name_alone() {
        // Claude has recorded nothing yet: the local name stands until it
        // does, and is outranked from then on.
        let mut app = App::new();
        let session = open_claude(&mut app, ClaudeLaunch::Resume("abc".into()));
        app.apply(Event::RenameSession {
            session: "abc".into(),
            title: "local".into(),
        });
        app.apply(Event::StatusChanged {
            session,
            status: SessionStatus::Idle,
        });

        let effects = app.apply(Event::SendClaudeCommand {
            session,
            command: ClaudeCommand::rename("from claude").expect("a name"),
            prompt: PromptInput::Empty,
        });
        assert!(
            effects.iter().all(|e| matches!(e, Effect::Write { .. })),
            "only the keystrokes: {effects:?}"
        );
        assert_eq!(title(&app), "local");
    }

    /// Report `job` in front of `session`'s shell.
    fn in_front(app: &mut App, session: SessionId, job: Option<ForegroundJob>) {
        app.apply(Event::ForegroundJobChanged { session, job });
    }

    fn a_claude_job() -> Option<ForegroundJob> {
        Some(ForegroundJob {
            pid: 42,
            started: None,
        })
    }

    #[test]
    fn a_claude_tab_whose_claude_has_exited_is_named_like_a_shell() {
        let mut app = App::new();
        let session = open_claude(&mut app, ClaudeLaunch::Fresh(None));
        in_front(&mut app, session, a_claude_job());
        assert!(app.tab_names_through_claude(0), "Claude in front");

        in_front(&mut app, session, None);
        app.apply(Event::StatusChanged {
            session,
            status: SessionStatus::Idle,
        });
        assert!(!app.tab_names_through_claude(0), "the shell is back");
        assert_eq!(
            app.claude_command_check(session, &PromptInput::Empty),
            Err(CommandRefusal::NotClaude),
            "nothing is typed into the shell left behind"
        );
        app.apply(Event::RenameTab {
            index: 0,
            title: "after claude".into(),
        });
        assert_eq!(title(&app), "after claude");

        in_front(&mut app, session, a_claude_job());
        assert!(
            app.tab_names_through_claude(0),
            "and Claude's again once something is back in front"
        );
    }

    #[test]
    fn a_pane_that_never_reports_its_foreground_still_names_through_claude() {
        // ConPTY reports no foreground at all, so the launch stands for it.
        let mut app = App::new();
        open_claude(&mut app, ClaudeLaunch::Fresh(None));
        assert!(app.tab_names_through_claude(0));
    }
}
