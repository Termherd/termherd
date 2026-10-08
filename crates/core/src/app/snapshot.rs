//! The filterable workspace snapshot builder (the perception rung).
//!
//! Assembles a [`WorkspaceSnapshot`] from the state `App` owns (workspace,
//! sidebar, sessions) plus the adapter-injected [`SnapshotInputs`] (config,
//! terminal text), shaped by a [`SnapshotFilter`]. Pure — no I/O, no panic.

use crate::browser::{SessionRecord, session_matches};
use crate::snapshot::{
    ClaudeIdentity, ConfigSummary, FocusRef, PaneSnapshot, ProjectSnapshot, Section,
    SidebarSnapshot, SnapshotFilter, SnapshotInputs, TabSnapshot, TerminalScope, WorkspaceSnapshot,
    tail_lines,
};
use std::collections::BTreeMap;
use termherd_claude::color::ClaudeColor;
use termherd_claude::session_file::SessionFile;

use super::*;

impl App {
    /// Build the workspace snapshot the caller asked for. Structural sections
    /// come from `self`; the config and terminal text ride in on `inputs`
    /// (the adapters own them). `filter` decides which sections and how much
    /// terminal text — light by default.
    ///
    /// A requested section still needs its adapter input to appear:
    /// [`Section::Config`] yields config only when `inputs.config` is present
    /// (the shell injects it exactly when the filter asks for it) — a missing
    /// input drops the section rather than erroring. Sidebar/tabs/focus need no
    /// input and always build when requested.
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
            tabs: filter
                .includes(Section::Tabs)
                .then(|| self.tab_snapshots(focus.tab, inputs)),
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
    ///
    /// Counts are computed over *borrowed* records — the same union, search +
    /// archive predicate and row order as [`Self::visible_projects`], but
    /// without cloning the digests it materialises for rendering. The shared
    /// parts go through `sidebar_row_shown` / `sidebar_row_order` rather than
    /// being restated here.
    fn sidebar_snapshot(&self) -> SidebarSnapshot {
        let needle = self.sidebar.search.trim().to_lowercase();
        let titles_only = self.sidebar.search_titles_only;
        let mut rows: Vec<(ProjectSnapshot, &[SessionRecord])> = self
            .merged_rows()
            .into_iter()
            .filter_map(|(path, sessions)| {
                // A path hit keeps the group whole (like `filter_rows`);
                // otherwise only content/title matches count. Then archived rows
                // drop unless shown. An empty needle keeps every session.
                let path_hit = needle.is_empty() || path.to_lowercase().contains(&needle);
                // The two filters run in the rendering path's order, and the
                // order is the whole point: the search decides whether the row
                // matched, the archive knob only decides what it *shows*. Read
                // off the archived-filtered count, a declared repo whose one
                // search hit was archived dropped out of the snapshot while the
                // screen still drew it.
                let matched = path_hit
                    || sessions
                        .iter()
                        .any(|s| session_matches(s, &needle, titles_only));
                let session_count = sessions
                    .iter()
                    .filter(|s| path_hit || session_matches(s, &needle, titles_only))
                    .filter(|s| self.sidebar.show_archived || !self.is_archived(&s.session_id))
                    .count();
                // The search applies to a declared repo like any other: it earns
                // its row by matching, then keeps it by being declared.
                (matched && self.sidebar_row_shown(path, session_count)).then(|| {
                    (
                        ProjectSnapshot {
                            path: path.to_owned(),
                            session_count,
                            collapsed: self.sidebar_row_collapsed(path, session_count),
                            declared: self.is_repo_declared(path),
                        },
                        sessions,
                    )
                })
            })
            .collect();
        rows.sort_by_key(|(project, unfiltered)| self.sidebar_row_order(&project.path, unfiltered));
        let projects: Vec<ProjectSnapshot> = rows.into_iter().map(|(project, _)| project).collect();
        SidebarSnapshot {
            hidden: self.sidebar.hidden,
            search: self.sidebar.search.clone(),
            search_titles_only: self.sidebar.search_titles_only,
            show_archived: self.sidebar.show_archived,
            projects,
        }
    }

    /// Each open tab with its panes (in pane order), addressed by stable handle.
    /// `active_tab` is the already-resolved focus pointer, so the active-tab
    /// invariant lives in one place ([`Self::snapshot`]).
    fn tab_snapshots(
        &self,
        active_tab: Option<usize>,
        inputs: &SnapshotInputs,
    ) -> Vec<TabSnapshot> {
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
                    .filter_map(|id| {
                        self.sessions.get(id).map(|session| {
                            pane_snapshot(
                                session,
                                &inputs.session_files,
                                self.session_color(session.id),
                            )
                        })
                    })
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
fn pane_snapshot(
    session: &LiveSession,
    session_files: &BTreeMap<u32, SessionFile>,
    color: Option<ClaudeColor>,
) -> PaneSnapshot {
    PaneSnapshot {
        handle: session.id.0.get(),
        kind: session.launch.kind(),
        cwd: session.cwd.clone(),
        status: session.status,
        identity: claude_identity(session, session_files),
        color,
    }
}

/// Who the Claude in `session` is, if the job in front of its shell is one.
/// The session file is the proof, not the kind the pane was launched as: a
/// Claude pane outlives its Claude (a `vim` after it is no Claude), and a
/// shell pane can run one typed by hand. The one place that decides it, so
/// the snapshot and `list_sessions` cannot disagree.
#[must_use]
pub fn claude_identity(
    session: &LiveSession,
    session_files: &BTreeMap<u32, SessionFile>,
) -> ClaudeIdentity {
    let job = session.foreground.as_ref();
    identity_of(job, job.and_then(|job| session_files.get(&job.pid)))
}

/// The identity `file` proves for `job`, or none. Pure over the two, so a
/// caller holding a cached file decides it exactly as the snapshot does.
#[must_use]
pub(super) fn identity_of(
    job: Option<&ForegroundJob>,
    file: Option<&SessionFile>,
) -> ClaudeIdentity {
    let (Some(job), Some(file)) = (job, file) else {
        return ClaudeIdentity::default();
    };
    if !proves(job, file) {
        return ClaudeIdentity::default();
    }
    ClaudeIdentity {
        pid: Some(job.pid),
        peer_name: file.name.clone(),
        session_id: file.session_id.clone(),
    }
}

/// Whether `file` was written by `job`, the process in front of a pane.
pub(super) fn proves(job: &ForegroundJob, file: &SessionFile) -> bool {
    // A crashed Claude leaves its file behind, and its pid free for whatever
    // the OS starts next: only the writer's own start time tells them apart.
    file.pid == job.pid && job.started.is_some() && job.started == file.proc_start
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;
    use crate::snapshot::{
        ConfigInput, PaneSnapshot, Section, SessionKind, SnapshotFilter, SnapshotInputs,
        TerminalScope,
    };
    use crate::workspace::SplitDir;
    use termherd_claude::session_file::SessionFile;

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
            launch: Launch::Claude(ClaudeLaunch::Fresh(None)),
            title: title.to_owned(),
            placement: Placement::Foreground,
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
        assert!(!project.declared, "this one came from the scan");
    }

    #[test]
    fn a_declared_repo_reaches_the_snapshot_with_no_sessions() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("s0", "/scanned", "one")]));
        app.apply(Event::DeclareRepo("/fresh".into()));

        let snap = app.snapshot(
            &only_sections(&[Section::Sidebar]),
            &SnapshotInputs::default(),
        );
        let sidebar = snap.sidebar.expect("sidebar was requested");
        let rows: Vec<(&str, usize, bool)> = sidebar
            .projects
            .iter()
            .map(|p| (p.path.as_str(), p.session_count, p.declared))
            .collect();
        // Same union, same order as the rendered sidebar: the declaration
        // outranks activity until it has a session of its own.
        assert_eq!(rows, vec![("/fresh", 0, true), ("/scanned", 1, false)]);
    }

    #[test]
    fn the_snapshot_rows_match_the_rendered_ones_declarations_included() {
        // The snapshot used to restate the sidebar's filter and order in its
        // own words, with a doc-comment asking the next reader to keep them in
        // step. This is that request, made checkable.
        let mut app = App::new();
        // Distinct mtimes, or every activity key is equal and the ordering
        // half of this assertion cannot fail — which is how it first shipped.
        let at = |id: &str, path: &str, secs: u64, summary: &str| {
            let mut r = record(id, path, summary);
            r.modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs));
            r
        };
        app.apply(Event::ScanCompleted(vec![
            at("s0", "/busy", 300, "the newest one"),
            at("s1", "/busy", 10, "an elderly one"),
            at("s2", "/quiet", 200, "an elderly one"),
        ]));
        app.apply(Event::DeclareRepo("/fresh".into()));
        app.apply(Event::ToggleRepoStar("/quiet".into()));
        // A declared repo whose only search hit is *archived*. The two filters
        // are not interchangeable — the search decides whether the row matched,
        // the archive knob only what it shows — and reading the match off the
        // archived-filtered count dropped this row from the snapshot while the
        // screen went on drawing it.
        app.apply(Event::ScanCompleted(vec![
            at("s0", "/busy", 300, "the newest one"),
            at("s1", "/busy", 10, "an elderly one"),
            at("s2", "/quiet", 200, "an elderly one"),
            at("s3", "/dusty", 100, "an elderly one"),
        ]));
        app.apply(Event::DeclareRepo("/dusty".into()));
        app.apply(Event::ToggleArchive("s3".into()));

        // "elderly" excludes `/busy`'s leading session, so a key taken from the
        // filtered set would reorder the two — on one side and not the other.
        for search in ["", "elderly", "e", "no-such-thing"] {
            app.apply(Event::SearchChanged(search.into()));
            let snap = app.snapshot(
                &only_sections(&[Section::Sidebar]),
                &SnapshotInputs::default(),
            );
            let snapped: Vec<String> = snap
                .sidebar
                .expect("sidebar was requested")
                .projects
                .into_iter()
                .map(|p| p.path)
                .collect();
            let rendered: Vec<String> = app
                .visible_projects()
                .into_iter()
                .map(|group| group.path)
                .collect();
            assert_eq!(snapped, rendered, "search {search:?}");
        }
    }

    #[test]
    fn an_empty_rows_fold_is_reported_as_the_screen_draws_it() {
        // A row with no session list cannot be folded, and the renderer draws
        // it expanded. Masking that in the view alone left the snapshot
        // reporting a fold the screen contradicted.
        let mut app = App::new();
        app.apply(Event::DeclareRepo("/fresh".into()));
        app.apply(Event::ToggleCollapsed("/fresh".into()));
        assert!(app.is_collapsed("/fresh"), "the raw flag is set");

        let collapsed = |app: &App| {
            app.snapshot(
                &only_sections(&[Section::Sidebar]),
                &SnapshotInputs::default(),
            )
            .sidebar
            .expect("sidebar")
            .projects
            .iter()
            .find(|p| p.path == "/fresh")
            .expect("the declared row")
            .collapsed
        };
        assert!(!collapsed(&app), "nothing to fold, so nothing is folded");

        // The day it gains a session there *is* a list, and the fold applies.
        app.apply(Event::ScanCompleted(vec![record("s1", "/fresh", "work")]));
        assert!(collapsed(&app));
    }

    #[test]
    fn sidebar_counts_and_order_match_visible_projects_under_search() {
        // The cheap borrowed count must stay faithful to the (cloning)
        // visible_projects it mirrors — this guards the two against drift.
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("a", "/p", "login bug"),
            record("b", "/p", "logout flow"),
            record("c", "/q", "unrelated"),
        ]));
        app.apply(Event::SearchChanged("log".into()));

        let snap = app.snapshot(
            &only_sections(&[Section::Sidebar]),
            &SnapshotInputs::default(),
        );
        let got: Vec<(String, usize)> = snap
            .sidebar
            .expect("sidebar")
            .projects
            .iter()
            .map(|project| (project.path.clone(), project.session_count))
            .collect();
        let expected: Vec<(String, usize)> = app
            .visible_projects()
            .iter()
            .map(|group| (group.path.clone(), group.sessions.len()))
            .collect();
        assert_eq!(got, expected, "borrowed counts mirror visible_projects");
        // "/q unrelated" doesn't match "log", so only "/p" survives with 2.
        assert_eq!(got, vec![("/p".to_owned(), 2)]);
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
    fn a_panes_directory_is_the_one_its_shell_is_in_not_the_one_it_started_in() {
        // The field documents itself as the path the session runs in, and an
        // agent builds relative commands from it. Reporting the launch
        // directory after a `cd` sends that agent to the wrong place with no
        // signal that anything moved.
        let mut app = App::new();
        let handle = launch_claude_in(&mut app, "/proj", "work");
        let session = SessionId(std::num::NonZeroU64::new(handle).expect("nonzero"));
        app.apply(Event::SessionCwdChanged {
            session,
            cwd: "/proj/crates/pty".into(),
        });

        let snap = app.snapshot(&only_sections(&[Section::Tabs]), &SnapshotInputs::default());
        let tabs = snap.tabs.expect("tabs were requested");
        assert_eq!(
            tabs[0].panes[0].cwd.as_deref(),
            Some("/proj/crates/pty"),
            "the snapshot must follow the shell, not the launch"
        );
    }

    fn session(handle: u64) -> SessionId {
        SessionId(std::num::NonZeroU64::new(handle).expect("nonzero"))
    }

    const STARTED: &str = "Wed Oct  7 06:48:07 2026";

    fn session_file(pid: u32, name: &str, session_id: &str) -> SessionFile {
        SessionFile {
            pid,
            name: Some(name.to_owned()),
            session_id: Some(session_id.to_owned()),
            proc_start: Some(STARTED.to_owned()),
        }
    }

    /// The job a file written by `session_file(pid, ..)` describes.
    fn job(pid: u32) -> ForegroundJob {
        ForegroundJob {
            pid,
            started: Some(STARTED.to_owned()),
        }
    }

    fn panes(app: &App, inputs: &SnapshotInputs) -> Vec<PaneSnapshot> {
        let snap = app.snapshot(&only_sections(&[Section::Tabs]), inputs);
        snap.tabs
            .expect("tabs were requested")
            .into_iter()
            .flat_map(|tab| tab.panes)
            .collect()
    }

    #[test]
    fn a_claude_pane_reports_its_pid_and_the_identity_its_session_file_names() {
        let mut app = App::new();
        let handle = launch_claude_in(&mut app, "/proj", "work");
        app.apply(Event::ForegroundJobChanged {
            session: session(handle),
            job: Some(job(4399)),
        });
        let inputs = SnapshotInputs {
            session_files: BTreeMap::from([(4399, session_file(4399, "proj-35", "7eff"))]),
            ..SnapshotInputs::default()
        };

        let pane = &panes(&app, &inputs)[0];
        assert_eq!(pane.identity.pid, Some(4399));
        assert_eq!(pane.identity.peer_name.as_deref(), Some("proj-35"));
        assert_eq!(pane.identity.session_id.as_deref(), Some("7eff"));
    }

    #[test]
    fn a_pane_reports_the_colour_its_transcript_set_and_a_shell_none() {
        let minted = "0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f9012";
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Claude(ClaudeLaunch::Fresh(Some(minted.into()))),
            title: "work".into(),
            placement: Placement::Foreground,
        }));
        launch(&mut app, "shell");
        let mut coloured = record(minted, "/proj", "prompt");
        coloured.digest.agent_color = Some(ClaudeColor::Yellow);
        app.apply(Event::ScanCompleted(vec![coloured]));

        let panes = panes(&app, &SnapshotInputs::default());
        assert_eq!(panes[0].color, Some(ClaudeColor::Yellow));
        assert_eq!(panes[1].color, None);
    }

    #[test]
    fn a_job_with_no_session_file_is_not_reported_as_claude() {
        // A Claude pane whose Claude quit, now running `vim`: its kind still
        // says Claude, the pid in front is not one.
        let mut app = App::new();
        let handle = launch_claude_in(&mut app, "/proj", "work");
        app.apply(Event::ForegroundJobChanged {
            session: session(handle),
            job: Some(job(4399)),
        });

        let pane = &panes(&app, &SnapshotInputs::default())[0];
        assert_eq!(pane.identity, ClaudeIdentity::default());
    }

    #[test]
    fn the_shell_back_in_front_clears_the_pid_and_the_identity() {
        let mut app = App::new();
        let handle = launch_claude_in(&mut app, "/proj", "work");
        app.apply(Event::ForegroundJobChanged {
            session: session(handle),
            job: Some(job(4399)),
        });
        app.apply(Event::ForegroundJobChanged {
            session: session(handle),
            job: None,
        });
        // A stale file for the old pid must not resurface once Claude left.
        let inputs = SnapshotInputs {
            session_files: BTreeMap::from([(4399, session_file(4399, "proj-35", "7eff"))]),
            ..SnapshotInputs::default()
        };

        let pane = &panes(&app, &inputs)[0];
        assert_eq!(
            (pane.identity.pid, pane.identity.peer_name.as_deref()),
            (None, None),
            "no job in front means no Claude to name"
        );
    }

    #[test]
    fn a_claude_typed_into_a_shell_pane_is_identified_by_its_session_file() {
        let mut app = App::new();
        let handle = launch(&mut app, "a").0.get();
        app.apply(Event::ForegroundJobChanged {
            session: session(handle),
            job: Some(job(5000)),
        });
        let inputs = SnapshotInputs {
            session_files: BTreeMap::from([(5000, session_file(5000, "typed-claude", "s"))]),
            ..SnapshotInputs::default()
        };

        let pane = &panes(&app, &inputs)[0];
        assert_eq!(pane.kind, SessionKind::Shell);
        assert_eq!(
            (
                pane.identity.pid,
                pane.identity.peer_name.as_deref(),
                pane.identity.session_id.as_deref()
            ),
            (Some(5000), Some("typed-claude"), Some("s")),
        );
    }

    #[test]
    fn two_claude_panes_in_one_directory_keep_their_own_identities() {
        // Same cwd, so nothing a pane already reported told them apart: the
        // case that sent a peer message to the wrong session.
        let mut app = App::new();
        let first = launch_claude_in(&mut app, "/proj", "one");
        let second = launch_claude_in(&mut app, "/proj", "two");
        for (handle, pid) in [(first, 4001), (second, 4002)] {
            app.apply(Event::ForegroundJobChanged {
                session: session(handle),
                job: Some(job(pid)),
            });
        }
        let inputs = SnapshotInputs {
            session_files: BTreeMap::from([
                (4001, session_file(4001, "proj-1", "aaaa")),
                (4002, session_file(4002, "proj-2", "bbbb")),
            ]),
            ..SnapshotInputs::default()
        };

        let by_handle: BTreeMap<u64, Option<String>> = panes(&app, &inputs)
            .into_iter()
            .map(|pane| (pane.handle, pane.identity.peer_name))
            .collect();
        assert_eq!(by_handle[&first].as_deref(), Some("proj-1"));
        assert_eq!(by_handle[&second].as_deref(), Some("proj-2"));
    }

    #[test]
    fn a_pane_whose_pty_died_keeps_no_identity() {
        let mut app = App::new();
        let handle = launch_claude_in(&mut app, "/proj", "work");
        app.apply(Event::ForegroundJobChanged {
            session: session(handle),
            job: Some(job(4399)),
        });
        app.apply(Event::PtyExited {
            session: session(handle),
            clean: false,
        });
        // The pid is free to be reused, by another Claude among others.
        let inputs = SnapshotInputs {
            session_files: BTreeMap::from([(4399, session_file(4399, "someone-else", "x"))]),
            ..SnapshotInputs::default()
        };

        let pane = &panes(&app, &inputs)[0];
        assert_eq!(
            pane.status,
            SessionStatus::Exited,
            "an unclean exit keeps the pane"
        );
        assert_eq!(pane.identity, ClaudeIdentity::default());
    }

    #[test]
    fn a_file_left_by_a_dead_claude_names_no_process_that_reuses_its_pid() {
        let mut app = App::new();
        let handle = launch(&mut app, "a").0.get();
        // `vim`, handed the pid a crashed Claude held, under that Claude's file.
        app.apply(Event::ForegroundJobChanged {
            session: session(handle),
            job: Some(ForegroundJob {
                pid: 4399,
                started: Some("Thu Oct  8 09:12:44 2026".to_owned()),
            }),
        });
        let inputs = SnapshotInputs {
            session_files: BTreeMap::from([(4399, session_file(4399, "proj-35", "7eff"))]),
            ..SnapshotInputs::default()
        };

        assert_eq!(panes(&app, &inputs)[0].identity, ClaudeIdentity::default());
    }

    #[test]
    fn a_job_whose_start_is_unknown_is_not_vouched_for() {
        let mut app = App::new();
        let handle = launch(&mut app, "a").0.get();
        app.apply(Event::ForegroundJobChanged {
            session: session(handle),
            job: Some(ForegroundJob {
                pid: 4399,
                started: None,
            }),
        });
        // A file with no stamp either: two absences are not a match.
        let mut file = session_file(4399, "proj-35", "7eff");
        file.proc_start = None;
        let inputs = SnapshotInputs {
            session_files: BTreeMap::from([(4399, file)]),
            ..SnapshotInputs::default()
        };

        assert_eq!(panes(&app, &inputs)[0].identity, ClaudeIdentity::default());
    }

    #[test]
    fn a_job_change_for_an_unknown_session_is_ignored() {
        let mut app = App::new();
        let effects = app.apply(Event::ForegroundJobChanged {
            session: session(99),
            job: Some(job(1)),
        });
        assert!(effects.is_empty());
        assert!(panes(&app, &SnapshotInputs::default()).is_empty());
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

    /// A Claude pane with `job` in front, the shell having read `file` for it.
    fn pane_with(job: Option<ForegroundJob>, file: Option<SessionFile>) -> (App, SessionId) {
        let mut app = App::new();
        let id = session(launch_claude_in(&mut app, "/proj", "work"));
        app.apply(Event::ForegroundJobChanged { session: id, job });
        app.apply(Event::SessionFileRead { session: id, file });
        (app, id)
    }

    #[test]
    fn the_cached_session_file_names_the_claude_in_front() {
        let (app, id) = pane_with(
            Some(job(4399)),
            Some(session_file(4399, "termherd-b0", "s")),
        );
        assert_eq!(app.peer_name(id).as_deref(), Some("termherd-b0"));
    }

    #[test]
    fn a_file_read_for_an_earlier_job_names_nobody_once_another_runs() {
        let (mut app, id) = pane_with(
            Some(job(4399)),
            Some(session_file(4399, "knowledge-hub-35", "s")),
        );
        app.apply(Event::ForegroundJobChanged {
            session: id,
            job: Some(job(4500)),
        });
        assert_eq!(app.peer_name(id), None);
    }

    #[test]
    fn a_reread_that_finds_no_file_forgets_the_name() {
        let (mut app, id) = pane_with(
            Some(job(4399)),
            Some(session_file(4399, "termherd-b0", "s")),
        );
        app.apply(Event::SessionFileRead {
            session: id,
            file: None,
        });
        assert_eq!(app.peer_name(id), None);
    }

    #[test]
    fn no_job_in_front_names_nobody_whatever_file_was_read() {
        // Windows: ConPTY reports no foreground group, so there is no pid.
        let (app, id) = pane_with(None, Some(session_file(4399, "termherd-b0", "s")));
        assert_eq!(app.peer_name(id), None);
    }

    #[test]
    fn a_dead_claudes_file_on_a_reused_pid_names_nobody() {
        let stale = ForegroundJob {
            pid: 4399,
            started: Some("Thu Oct  8 09:00:00 2026".to_owned()),
        };
        let (app, id) = pane_with(Some(stale), Some(session_file(4399, "termherd-b0", "s")));
        assert_eq!(app.peer_name(id), None);
    }
}
