//! The filterable workspace snapshot builder (the perception rung).
//!
//! Assembles a [`WorkspaceSnapshot`] from the state `App` owns (workspace,
//! sidebar, sessions) plus the adapter-injected [`SnapshotInputs`] (config,
//! terminal text), shaped by a [`SnapshotFilter`]. Pure — no I/O, no panic.

use std::collections::BTreeMap;

use crate::snapshot::{
    ConfigSummary, FocusRef, PaneSnapshot, ProjectSnapshot, Section, SessionKind, SidebarSnapshot,
    SnapshotFilter, SnapshotInputs, TabSnapshot, TerminalScope, WorkspaceSnapshot, tail_lines,
};

use super::*;

impl App {
    /// Build the workspace snapshot the caller asked for. Structural sections
    /// come from `self`; the config and terminal text ride in on `inputs`
    /// (the adapters own them). `filter` decides which sections and how much
    /// terminal text — light by default.
    #[must_use]
    pub fn snapshot(&self, filter: &SnapshotFilter, inputs: &SnapshotInputs) -> WorkspaceSnapshot {
        let focus = FocusRef {
            tab: (!self.workspace.tabs.is_empty()).then_some(self.workspace.active),
            session: self.workspace.focused_session().map(|s| s.0.get()),
        };
        WorkspaceSnapshot {
            // Config folds the adapter-injected bits with the live font size the
            // core owns — carried only when the section was asked for.
            config: filter
                .includes(Section::Config)
                .then(|| self.config_summary(inputs))
                .flatten(),
            sidebar: filter
                .includes(Section::Sidebar)
                .then(|| self.sidebar_snapshot()),
            tabs: filter.includes(Section::Tabs).then(|| self.tab_snapshots()),
            terminals: self.scoped_terminals(filter, inputs, focus.session),
            focus,
        }
    }

    /// Fold the adapter-injected config bits with the live font size the core
    /// owns (base + zoom). `None` when the adapter injected no config.
    fn config_summary(&self, inputs: &SnapshotInputs) -> Option<ConfigSummary> {
        inputs.config.as_ref().map(|input| ConfigSummary {
            font_size: self.font_size(),
            terminal_scheme: input.terminal_scheme.clone(),
            record_fps: input.record_fps,
            record_scale: input.record_scale,
            keymap_overrides: input.keymap_overrides,
        })
    }

    /// The light sidebar view: the filter knobs plus one row per *visible*
    /// project (its path, visible-session count, and fold state). The full
    /// per-session browser rows are a deeper read, deliberately out.
    fn sidebar_snapshot(&self) -> SidebarSnapshot {
        let projects = self
            .visible_projects()
            .iter()
            .map(|group| ProjectSnapshot {
                path: group.path.clone(),
                session_count: group.sessions.len(),
                collapsed: self.is_collapsed(&group.path),
            })
            .collect();
        SidebarSnapshot {
            hidden: self.sidebar.hidden,
            search: self.sidebar.search.clone(),
            search_titles_only: self.sidebar.search_titles_only,
            show_archived: self.sidebar.show_archived,
            projects,
        }
    }

    /// Each open tab with its panes (in pane order), addressed by stable handle.
    fn tab_snapshots(&self) -> Vec<TabSnapshot> {
        let active_tab = (!self.workspace.tabs.is_empty()).then_some(self.workspace.active);
        self.workspace
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| TabSnapshot {
                active: active_tab == Some(index),
                title: tab.display_title().to_owned(),
                status: self.tab_status(index),
                panes: tab
                    .sessions()
                    .iter()
                    // A pane always hosts a registered session (the workspace
                    // invariant); a stray id is dropped rather than panicked on.
                    .filter_map(|id| self.sessions.get(id).map(pane_snapshot))
                    .collect(),
            })
            .collect()
    }

    /// The terminal text the filter scopes in, each truncated to `text_lines`.
    /// A handle with no text available (its grid not injected) is simply absent.
    fn scoped_terminals(
        &self,
        filter: &SnapshotFilter,
        inputs: &SnapshotInputs,
        focused: Option<u64>,
    ) -> BTreeMap<u64, String> {
        let handles: Vec<u64> = match &filter.terminals {
            TerminalScope::None => return BTreeMap::new(),
            TerminalScope::Focused => focused.into_iter().collect(),
            TerminalScope::Only(handles) => handles.clone(),
        };
        handles
            .into_iter()
            .filter_map(|handle| {
                inputs
                    .terminals
                    .get(&handle)
                    .map(|text| (handle, tail_lines(text, filter.text_lines)))
            })
            .collect()
    }
}

/// One live session as a snapshot pane. Free function (not a method) — it reads
/// only the session, so it needs no `App`.
fn pane_snapshot(session: &LiveSession) -> PaneSnapshot {
    PaneSnapshot {
        handle: session.id.0.get(),
        kind: match session.launch {
            Launch::Shell => SessionKind::Shell,
            Launch::Claude { .. } => SessionKind::Claude,
        },
        cwd: session.cwd.clone(),
        status: session.status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;
    use crate::snapshot::{
        ConfigInput, Section, SessionKind, SnapshotFilter, SnapshotInputs, TerminalScope,
    };
    use crate::workspace::SplitDir;

    /// A filter for exactly the sections named, otherwise light (no terminal
    /// text).
    fn only_sections(sections: &[Section]) -> SnapshotFilter {
        SnapshotFilter {
            sections: sections.to_vec(),
            ..SnapshotFilter::default()
        }
    }

    /// Launch a Claude session in `cwd` and return its handle.
    fn launch_claude_in(app: &mut App, cwd: &str, title: &str) -> u64 {
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some(cwd.to_owned()),
            launch: Launch::Claude { resume: None },
            title: title.to_owned(),
        }));
        app.workspace
            .focused_session()
            .expect("a focused session")
            .0
            .get()
    }

    #[test]
    fn focus_reports_the_active_tab_and_focused_session() {
        let mut app = App::new();
        launch(&mut app, "first");
        let second = launch(&mut app, "second");
        let snap = app.snapshot(&SnapshotFilter::default(), &SnapshotInputs::default());
        assert_eq!(snap.focus.tab, Some(app.workspace.active));
        assert_eq!(
            snap.focus.session,
            Some(second.0.get()),
            "focus follows the newest launch"
        );
    }

    #[test]
    fn focus_is_empty_on_a_fresh_workspace() {
        let snap = App::new().snapshot(&SnapshotFilter::default(), &SnapshotInputs::default());
        assert_eq!(snap.focus, FocusRef::default());
    }

    #[test]
    fn config_section_carries_injected_bits_and_the_live_font_size() {
        let mut app = App::new();
        launch(&mut app, "a");
        let inputs = SnapshotInputs {
            config: Some(ConfigInput {
                terminal_scheme: Some("gruvbox-dark".into()),
                record_fps: 8,
                record_scale: 0.5,
                keymap_overrides: 2,
            }),
            ..SnapshotInputs::default()
        };
        let snap = app.snapshot(&only_sections(&[Section::Config]), &inputs);
        let config = snap.config.expect("config was requested and injected");
        // The adapter bits ride through unchanged...
        assert_eq!(config.terminal_scheme.as_deref(), Some("gruvbox-dark"));
        assert_eq!(config.record_fps, 8);
        assert_eq!(config.keymap_overrides, 2);
        // ...and the font size is stamped live from core, not injected.
        assert_eq!(config.font_size, app.font_size());
    }

    #[test]
    fn config_is_absent_when_the_section_is_not_requested() {
        let inputs = SnapshotInputs {
            config: Some(ConfigInput {
                terminal_scheme: None,
                record_fps: 8,
                record_scale: 0.5,
                keymap_overrides: 0,
            }),
            ..SnapshotInputs::default()
        };
        // The section is off, so even an injected config must not appear.
        let snap = App::new().snapshot(&only_sections(&[Section::Tabs]), &inputs);
        assert_eq!(snap.config, None);
    }

    #[test]
    fn sidebar_section_lists_projects_with_counts_and_fold_state() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("s0", "/p", "one"),
            record("s1", "/p", "two"),
        ]));
        app.apply(Event::ToggleCollapsed("/p".into()));

        let snap = app.snapshot(
            &only_sections(&[Section::Sidebar]),
            &SnapshotInputs::default(),
        );
        let sidebar = snap.sidebar.expect("sidebar was requested");
        assert!(!sidebar.hidden);
        assert_eq!(sidebar.projects.len(), 1);
        let project = &sidebar.projects[0];
        assert_eq!(project.path, "/p");
        assert_eq!(project.session_count, 2, "both sessions are visible");
        assert!(project.collapsed, "the project was folded shut");
    }

    #[test]
    fn tabs_section_reports_each_pane_with_handle_kind_cwd_and_status() {
        let mut app = App::new();
        let claude = launch_claude_in(&mut app, "/proj", "work");
        app.apply(Event::StatusChanged {
            session: SessionId(std::num::NonZeroU64::new(claude).expect("nonzero")),
            status: SessionStatus::Busy,
        });
        // Split: the sibling is a plain shell inheriting the cwd.
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        let shell = app
            .workspace
            .focused_session()
            .expect("focused pane")
            .0
            .get();

        let snap = app.snapshot(&only_sections(&[Section::Tabs]), &SnapshotInputs::default());
        let tabs = snap.tabs.expect("tabs were requested");
        assert_eq!(tabs.len(), 1);
        let tab = &tabs[0];
        assert!(tab.active);
        assert_eq!(tab.title, "work");
        assert_eq!(tab.panes.len(), 2, "a split hosts two panes");

        let claude_pane = &tab.panes[0];
        assert_eq!(claude_pane.handle, claude);
        assert_eq!(claude_pane.kind, SessionKind::Claude);
        assert_eq!(claude_pane.cwd.as_deref(), Some("/proj"));
        assert_eq!(claude_pane.status, SessionStatus::Busy);

        let shell_pane = &tab.panes[1];
        assert_eq!(shell_pane.handle, shell);
        assert_eq!(shell_pane.kind, SessionKind::Shell);
        assert_eq!(
            shell_pane.cwd.as_deref(),
            Some("/proj"),
            "the split inherits cwd"
        );
    }

    #[test]
    fn empty_workspace_tabs_section_is_present_but_empty() {
        let snap =
            App::new().snapshot(&only_sections(&[Section::Tabs]), &SnapshotInputs::default());
        assert_eq!(
            snap.tabs,
            Some(Vec::new()),
            "the section is built (Some) but holds no tabs"
        );
    }

    #[test]
    fn terminals_are_empty_by_default_even_when_text_is_available() {
        let mut app = App::new();
        let handle = launch(&mut app, "a").0.get();
        let inputs = SnapshotInputs {
            terminals: BTreeMap::from([(handle, "some output".to_owned())]),
            ..SnapshotInputs::default()
        };
        // Default scope is None: the light read carries no terminal text.
        let snap = app.snapshot(&SnapshotFilter::default(), &inputs);
        assert!(snap.terminals.is_empty());
    }

    #[test]
    fn focused_scope_returns_only_the_focused_pane_truncated() {
        let mut app = App::new();
        let handle = launch(&mut app, "a").0.get();
        let text = (1..=100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let inputs = SnapshotInputs {
            terminals: BTreeMap::from([(handle, text)]),
            ..SnapshotInputs::default()
        };
        let filter = SnapshotFilter {
            terminals: TerminalScope::Focused,
            text_lines: 3,
            ..SnapshotFilter::default()
        };
        let snap = app.snapshot(&filter, &inputs);
        assert_eq!(
            snap.terminals.get(&handle).map(String::as_str),
            Some("line 98\nline 99\nline 100"),
            "only the focused pane, truncated to the last 3 lines"
        );
        assert_eq!(snap.terminals.len(), 1);
    }

    #[test]
    fn only_scope_returns_just_the_named_handles() {
        let mut app = App::new();
        let first = launch(&mut app, "a").0.get();
        let second = launch(&mut app, "b").0.get();
        let inputs = SnapshotInputs {
            terminals: BTreeMap::from([(first, "aaa".to_owned()), (second, "bbb".to_owned())]),
            ..SnapshotInputs::default()
        };
        let filter = SnapshotFilter {
            terminals: TerminalScope::Only(vec![second]),
            ..SnapshotFilter::default()
        };
        let snap = app.snapshot(&filter, &inputs);
        assert_eq!(
            snap.terminals.keys().copied().collect::<Vec<_>>(),
            vec![second]
        );
        assert_eq!(snap.terminals.get(&second).map(String::as_str), Some("bbb"));
    }
}
