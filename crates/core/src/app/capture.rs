//! The capture snapshot for the AI dev loop (`F-capture`, rung 0/1).

use crate::capture::{CaptureDump, CaptureTab};

use super::*;

impl App {
    /// Assemble the capture snapshot for the AI dev loop. Pure: it reads
    /// the workspace and live-session state and folds in the focused terminal's
    /// text the shell supplied (the grid lives in the `pty` adapter). The result
    /// is the diffable rung-0 payload; the shell adds the rung-1 PNG.
    #[must_use]
    pub fn build_capture(&self, focused_pty_text: Option<String>) -> CaptureDump {
        let active_tab = (!self.workspace.tabs.is_empty()).then_some(self.workspace.active);
        let focused = self.workspace.focused_session();
        let tabs = self
            .workspace
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let active = active_tab == Some(index);
                CaptureTab {
                    active,
                    title: tab.display_title().to_owned(),
                    status: self.tab_status(index),
                    sessions: tab.sessions().into_iter().map(|s| s.0.get()).collect(),
                    // Only the active tab has a live focus to report.
                    focus_session: focused.filter(|_| active).map(|s| s.0.get()),
                }
            })
            .collect();
        CaptureDump {
            active_tab,
            tabs,
            focused_pty: focused_pty_text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;
    use crate::workspace::SplitDir;

    #[test]
    fn capture_snapshots_tabs_focus_status_and_pty_text() {
        let mut app = App::new();
        let first = launch(&mut app, "proj $");
        let second = launch(&mut app, "repo 🤖");
        app.apply(Event::StatusChanged {
            session: second,
            status: SessionStatus::Busy,
        });

        let effects = app.apply(Event::Capture {
            focused_pty_text: Some("$ cargo test\nok".to_owned()),
        });
        let dump = capture_dump(&effects);

        // The active tab is the last launched one, carrying its focus.
        assert_eq!(dump.active_tab, Some(1));
        assert_eq!(dump.tabs.len(), 2);
        assert_eq!(dump.focused_pty.as_deref(), Some("$ cargo test\nok"));

        let tab0 = &dump.tabs[0];
        assert!(!tab0.active);
        assert_eq!(tab0.title, "proj $");
        assert_eq!(tab0.status, Some(SessionStatus::Starting));
        assert_eq!(tab0.sessions, vec![first.0.get()]);
        assert_eq!(
            tab0.focus_session, None,
            "only the active tab reports focus"
        );

        let tab1 = &dump.tabs[1];
        assert!(tab1.active);
        assert_eq!(tab1.title, "repo 🤖");
        assert_eq!(tab1.status, Some(SessionStatus::Busy));
        assert_eq!(tab1.sessions, vec![second.0.get()]);
        assert_eq!(tab1.focus_session, Some(second.0.get()));
    }

    #[test]
    fn capture_reports_a_tabs_custom_title_not_its_derived_one() {
        let mut app = App::new();
        launch(&mut app, "derived");
        app.apply(Event::RenameTab {
            index: 0,
            title: "My work".into(),
        });

        let effects = app.apply(Event::Capture {
            focused_pty_text: None,
        });
        let dump = capture_dump(&effects);
        // The dump must match what the user sees on the chip, or an AI reading
        // the state would name the tab wrong.
        assert_eq!(dump.tabs[0].title, "My work");
    }

    #[test]
    fn capture_on_an_empty_workspace_has_no_active_tab() {
        let mut app = App::new();
        let effects = app.apply(Event::Capture {
            focused_pty_text: None,
        });
        let dump = capture_dump(&effects);
        assert_eq!(dump.active_tab, None);
        assert!(dump.tabs.is_empty());
        assert_eq!(dump.focused_pty, None);
    }

    #[test]
    fn capture_lists_split_pane_membership_in_order() {
        // A split tab hosts several sessions; the dump records them in pane
        // order and points focus at the newest pane (layout/state proxy).
        let mut app = App::new();
        let base = launch(&mut app, "proj");
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        let split = app.workspace.focused_session().expect("focused split pane");

        let effects = app.apply(Event::Capture {
            focused_pty_text: None,
        });
        let dump = capture_dump(&effects);
        let tab = &dump.tabs[0];
        assert_eq!(tab.sessions, vec![base.0.get(), split.0.get()]);
        assert_eq!(tab.focus_session, Some(split.0.get()));
    }
}
