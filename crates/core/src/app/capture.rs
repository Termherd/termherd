//! The capture snapshot for the AI dev loop (`F-capture`, rung 0/1).

use crate::snapshot::{SnapshotFilter, SnapshotInputs, WorkspaceSnapshot};

use super::*;

impl App {
    /// Assemble the capture payload for the AI dev loop: the same
    /// [`WorkspaceSnapshot`] an MCP client reads, under the fixed
    /// [`SnapshotFilter::capture`] shape. The shell injects what it owns
    /// (`inputs`: the resolved config, the focused pane's text) and adds the
    /// rung-1 PNG; this stays the pure, diffable rung-0 payload.
    #[must_use]
    pub fn build_capture(&self, inputs: &SnapshotInputs) -> WorkspaceSnapshot {
        self.snapshot(&SnapshotFilter::capture(), inputs)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::app::testsupport::*;
    use crate::snapshot::ConfigInput;
    use crate::workspace::SplitDir;

    /// Adapter-injected inputs carrying a config block and the focused pane's
    /// text — what the shell hands in on a capture.
    fn inputs(focused: SessionId, text: &str) -> SnapshotInputs {
        SnapshotInputs {
            config: Some(ConfigInput {
                terminal_scheme: Some("gruvbox-dark".into()),
                record_fps: 8,
                record_scale: 0.5,
                keymap_overrides: 2,
            }),
            terminals: BTreeMap::from([(focused.0.get(), text.to_owned())]),
        }
    }

    #[test]
    fn capture_dumps_every_section_of_the_workspace_snapshot() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("s0", "/p", "work")]));
        let first = launch(&mut app, "proj $");
        let second = launch(&mut app, "repo 🤖");
        app.apply(Event::StatusChanged {
            session: second,
            status: SessionStatus::Busy,
        });

        let effects = app.apply(Event::Capture(inputs(second, "$ cargo test\nok")));
        let snapshot = captured_snapshot(&effects);

        // Focus: the active tab is the last launched one, carrying its session.
        assert_eq!(snapshot.focus.tab, Some(1));
        assert_eq!(snapshot.focus.session, Some(second.0.get()));
        // The three structural sections a capture always carries — the dump is
        // the *whole* app, not just the terminal.
        let config = snapshot.config.as_ref().expect("config section");
        assert_eq!(config.terminal_scheme.as_deref(), Some("gruvbox-dark"));
        let sidebar = snapshot.sidebar.as_ref().expect("sidebar section");
        assert_eq!(sidebar.projects.len(), 1);
        let tabs = snapshot.tabs.as_ref().expect("tabs section");
        assert_eq!(tabs.len(), 2);
        assert!(!tabs[0].active);
        assert_eq!(tabs[0].title, "proj $");
        assert_eq!(tabs[0].status, Some(SessionStatus::Starting));
        assert_eq!(tabs[0].panes[0].handle, first.0.get());
        assert!(tabs[1].active);
        assert_eq!(tabs[1].status, Some(SessionStatus::Busy));
    }

    #[test]
    fn capture_carries_the_focused_terminal_text_whole() {
        let mut app = App::new();
        let session = launch(&mut app, "proj");
        let long = (1..=500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        let effects = app.apply(Event::Capture(inputs(session, &long)));
        let snapshot = captured_snapshot(&effects);
        assert_eq!(
            snapshot.terminals.get(&session.0.get()).map(String::as_str),
            Some(long.as_str()),
            "the dev-loop dump keeps the focused grid untruncated"
        );
        assert_eq!(snapshot.terminals.len(), 1, "only the focused pane");
    }

    #[test]
    fn capture_ignores_text_for_panes_that_do_not_hold_focus() {
        let mut app = App::new();
        let base = launch(&mut app, "proj");
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        let split = app.workspace.focused_session().expect("focused split pane");
        let mut injected = inputs(split, "focused output");
        injected
            .terminals
            .insert(base.0.get(), "background output".to_owned());

        let effects = app.apply(Event::Capture(injected));
        let snapshot = captured_snapshot(&effects);
        assert_eq!(
            snapshot.terminals.keys().copied().collect::<Vec<_>>(),
            vec![split.0.get()]
        );
    }

    #[test]
    fn capture_reports_a_tabs_custom_title_not_its_derived_one() {
        let mut app = App::new();
        launch(&mut app, "derived");
        app.apply(Event::RenameTab {
            index: 0,
            title: "My work".into(),
        });

        let effects = app.apply(Event::Capture(SnapshotInputs::default()));
        let snapshot = captured_snapshot(&effects);
        // The dump must match what the user sees on the chip, or an AI reading
        // the state would name the tab wrong.
        assert_eq!(snapshot.tabs.as_ref().expect("tabs")[0].title, "My work");
    }

    #[test]
    fn capture_on_an_empty_workspace_has_no_focus_and_no_tabs() {
        let mut app = App::new();
        let effects = app.apply(Event::Capture(SnapshotInputs::default()));
        let snapshot = captured_snapshot(&effects);
        assert_eq!(snapshot.focus, crate::snapshot::FocusRef::default());
        assert_eq!(snapshot.tabs, Some(Vec::new()));
        assert!(snapshot.terminals.is_empty());
    }

    #[test]
    fn capture_lists_split_pane_membership_in_order() {
        // A split tab hosts several sessions; the dump records them in pane
        // order and points focus at the newest pane (layout/state proxy).
        let mut app = App::new();
        let base = launch(&mut app, "proj");
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        let split = app.workspace.focused_session().expect("focused split pane");

        let effects = app.apply(Event::Capture(SnapshotInputs::default()));
        let snapshot = captured_snapshot(&effects);
        let tab = &snapshot.tabs.as_ref().expect("tabs")[0];
        let handles: Vec<u64> = tab.panes.iter().map(|pane| pane.handle).collect();
        assert_eq!(handles, vec![base.0.get(), split.0.get()]);
        assert_eq!(snapshot.focus.session, Some(split.0.get()));
    }
}
