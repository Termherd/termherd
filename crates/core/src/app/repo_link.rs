//! A `termherd://open` link: bring forward the tab working in a repo, or open
//! a shell there.

use super::*;

impl App {
    /// Show the tab with a pane working in `repo`, under the link's spelling or
    /// the `resolved` one, or open a shell at `repo`.
    pub(super) fn open_repo(&mut self, repo: &str, resolved: Option<&str>) -> Vec<Effect> {
        let working = self
            .session_working_in(repo)
            .or_else(|| resolved.and_then(|real| self.session_working_in(real)));
        if let Some(session) = working {
            self.workspace.reveal_pane_of(session);
            return Vec::new();
        }
        let launch = Launch::Shell;
        self.launch(LaunchSpec {
            cwd: Some(repo.into()),
            title: self.tab_title(repo, &launch),
            launch,
            placement: Placement::Foreground,
        })
    }

    /// The focused session when it works in `repo`, else the first in strip
    /// order that does.
    fn session_working_in(&self, repo: &str) -> Option<SessionId> {
        let works_in_repo = |session: &SessionId| {
            self.sessions
                .get(session)
                .and_then(|live| live.cwd.as_deref())
                .is_some_and(|cwd| lies_within(cwd, repo))
        };
        self.workspace
            .focused_session()
            .filter(works_in_repo)
            .or_else(|| {
                self.workspace
                    .tabs
                    .iter()
                    .flat_map(crate::workspace::Tab::sessions)
                    .find(works_in_repo)
            })
    }
}

/// Whether `path` is `dir` or below it, compared by whole components, so
/// `/work/repo-2` is not inside `/work/repo`.
fn lies_within(path: &str, dir: &str) -> bool {
    let separators: &[char] = &['/', '\\'];
    let path = path.trim_end_matches(separators);
    let dir = dir.trim_end_matches(separators);
    path.strip_prefix(dir)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(separators))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::SplitDir;
    use proptest::prelude::*;

    /// Open a shell tab in `cwd`, in front.
    fn launch_in(app: &mut App, cwd: &str) -> SessionId {
        let effects = app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some(cwd.into()),
            launch: Launch::Shell,
            title: cwd.into(),
            placement: Placement::Foreground,
        }));
        match effects.as_slice() {
            [Effect::Spawn(spec)] | [Effect::Spawn(spec), Effect::TabShown] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    fn open_link(path: &str) -> Event {
        Event::OpenRepo {
            path: path.into(),
            resolved: None,
        }
    }

    #[test]
    fn a_pane_in_either_spelling_of_the_repo_works_in_it() {
        let mut app = App::new();
        let written = launch_in(&mut app, "/tmp/repo");
        let real = launch_in(&mut app, "/private/tmp/repo");
        launch_in(&mut app, "/work/other");
        let link = || Event::OpenRepo {
            path: "/tmp/repo".into(),
            resolved: Some("/private/tmp/repo".into()),
        };

        let effects = app.apply(link());
        assert!(spawned(&effects).is_empty(), "{effects:?}");
        assert_eq!(app.workspace.focused_session(), Some(written));

        app.apply(Event::CloseTab(0));
        let effects = app.apply(link());
        assert!(spawned(&effects).is_empty(), "{effects:?}");
        assert_eq!(app.workspace.focused_session(), Some(real));
    }

    #[test]
    fn a_new_shell_opens_where_the_link_said() {
        let mut app = App::new();
        let effects = app.apply(Event::OpenRepo {
            path: "/tmp/repo".into(),
            resolved: Some("/private/tmp/repo".into()),
        });
        let [spec] = spawned(&effects)[..] else {
            panic!("expected one spawn, got {effects:?}");
        };
        assert_eq!(spec.cwd.as_deref(), Some("/tmp/repo"));
    }

    fn spawned(effects: &[Effect]) -> Vec<&SpawnSpec> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Spawn(spec) => Some(spec),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_repo_a_tab_already_works_in_is_shown_not_reopened() {
        let mut app = App::new();
        let repo = launch_in(&mut app, "/work/repo");
        launch_in(&mut app, "/work/other");

        let effects = app.apply(open_link("/work/repo"));

        assert!(
            matches!(effects.as_slice(), [Effect::TabShown]),
            "{effects:?}"
        );
        assert_eq!(app.workspace.focused_session(), Some(repo));
        assert_eq!(app.workspace.tabs.len(), 2);
    }

    #[test]
    fn a_session_in_a_subdirectory_works_in_the_repo() {
        let mut app = App::new();
        let deep = launch_in(&mut app, "/work/repo/crates/core");
        launch_in(&mut app, "/work/other");

        let effects = app.apply(open_link("/work/repo"));

        assert!(spawned(&effects).is_empty(), "{effects:?}");
        assert_eq!(app.workspace.focused_session(), Some(deep));
    }

    #[test]
    fn a_sibling_sharing_a_prefix_is_another_repo() {
        let mut app = App::new();
        let sibling = launch_in(&mut app, "/work/repo-2");

        let effects = app.apply(open_link("/work/repo"));

        let [spec] = spawned(&effects)[..] else {
            panic!("expected one spawn, got {effects:?}");
        };
        assert_eq!(spec.cwd.as_deref(), Some("/work/repo"));
        assert_ne!(app.workspace.focused_session(), Some(sibling));
    }

    #[test]
    fn the_pane_working_in_the_repo_takes_the_focus_inside_a_split() {
        let mut app = App::new();
        let left = launch_in(&mut app, "/work/other");
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        let right = app
            .workspace
            .focused_session()
            .expect("the split focuses its pane");
        assert_ne!(left, right);
        app.apply(Event::SessionCwdChanged {
            session: right,
            cwd: "/work/repo".into(),
        });
        app.apply(Event::FocusPane(left));
        launch_in(&mut app, "/work/third");

        let effects = app.apply(open_link("/work/repo"));

        assert!(spawned(&effects).is_empty(), "{effects:?}");
        assert_eq!(app.workspace.focused_session(), Some(right));
    }

    #[test]
    fn a_parked_tab_in_the_repo_is_unparked_and_shown() {
        let mut app = App::new();
        let repo = launch_in(&mut app, "/work/repo");
        launch_in(&mut app, "/work/other");
        app.apply(Event::ToggleParkTab(0));

        app.apply(open_link("/work/repo"));

        assert_eq!(app.workspace.focused_session(), Some(repo));
        let index = app.workspace.tab_of(repo).expect("still open");
        assert!(!app.workspace.tabs[index].is_parked());
    }

    #[test]
    fn a_repo_no_tab_works_in_opens_a_shell_there_in_front() {
        let mut app = App::new();
        launch_in(&mut app, "/work/other");

        let effects = app.apply(open_link("/work/repo"));

        let [spec] = spawned(&effects)[..] else {
            panic!("expected one spawn, got {effects:?}");
        };
        assert!(
            matches!(spec.launch, Launch::Shell),
            "a link never launches Claude"
        );
        assert_eq!(spec.cwd.as_deref(), Some("/work/repo"));
        assert_eq!(app.workspace.focused_session(), Some(spec.session));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::TabShown))
        );
    }

    #[test]
    fn a_link_into_an_empty_workspace_still_reports_the_tab_shown() {
        // With no tab shown before, the shown-tab diff alone reports nothing,
        // and the shell would neither size nor focus the new tab.
        let mut app = App::new();

        let effects = app.apply(open_link("/work/repo"));

        assert_eq!(spawned(&effects).len(), 1, "{effects:?}");
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::TabShown))
        );
    }

    #[test]
    fn a_trailing_slash_on_a_session_cwd_still_matches() {
        let mut app = App::new();
        let repo = launch_in(&mut app, "/work/repo/");
        launch_in(&mut app, "/work/other");

        let effects = app.apply(open_link("/work/repo"));

        assert!(spawned(&effects).is_empty(), "{effects:?}");
        assert_eq!(app.workspace.focused_session(), Some(repo));
    }

    proptest! {
        #[test]
        fn a_link_never_types_into_a_session_nor_launches_claude(
            cwds in proptest::collection::vec("/[ab]{1,2}(/[ab]{1,2}){0,2}", 0..5),
            repo in "/[ab]{1,2}(/[ab]{1,2}){0,2}",
        ) {
            let mut app = App::new();
            for cwd in &cwds {
                launch_in(&mut app, cwd);
            }
            let effects = app.apply(open_link(&repo));
            for effect in &effects {
                prop_assert!(!matches!(effect, Effect::Write { .. }), "{:?}", effect);
                if let Effect::Spawn(spec) = effect {
                    prop_assert!(matches!(spec.launch, Launch::Shell), "{:?}", spec.launch);
                }
            }
            prop_assert!(spawned(&effects).len() <= 1);
        }
    }
}
