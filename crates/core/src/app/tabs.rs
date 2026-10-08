//! Tab lifecycle: close, the reopen-closed stack, and the derived tab title /
//! status / record read models.

use crate::browser::{SessionRecord, project_label};
use crate::snapshot::SessionKind;
use termherd_claude::color::ClaudeColor;

use super::*;

/// How many closed tabs the reopen stack remembers. Walking back further
/// than this is rare enough that the unbounded-growth risk outweighs it.
const MAX_CLOSED_TABS: usize = 16;

/// Enough of a closed tab to recreate it on reopen: the kind it ran, the
/// directory it ran in, and the label it carried. A split tab is reduced to its
/// first pane — reopen restores a single terminal, not the whole pane tree.
#[derive(Debug, Clone)]
pub struct ClosedTab {
    pub title: String,
    /// The manual name overlaid on the derived title when the tab was closed, if
    /// any — restored on reopen so a rename round-trips, not just the digest.
    pub custom_title: Option<String>,
    pub cwd: Option<String>,
    pub launch: Launch,
}

/// Who a pane's colour belongs to, and so how it is changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorKeeper {
    /// Claude Code records it; termherd changes it by typing `/color`.
    Claude,
    /// Claude knows nothing of the pane; termherd stores it on the tab.
    Termherd,
}

impl App {
    /// Close a tab (FR5): drop its sessions from the live registry and ask the
    /// runtime to kill each PTY. An out-of-range index yields no effects.
    /// Snapshots the tab onto the reopen stack first, so the close can be
    /// undone before its sessions are forgotten.
    pub(super) fn close_tab(&mut self, index: usize) -> Vec<Effect> {
        self.remember_closed_tab(index);
        let sessions = self.workspace.close_tab(index);
        for id in &sessions {
            self.sessions.remove(id);
        }
        sessions.into_iter().map(Effect::Kill).collect()
    }

    /// Push the tab at `index` onto the reopen stack, capturing the kind,
    /// directory and label needed to recreate it. Reduced to the tab's first
    /// pane — reopen restores one terminal, not a whole split. A no-op for an
    /// out-of-range index or a tab whose first session is no longer live.
    pub(super) fn remember_closed_tab(&mut self, index: usize) {
        let Some(tab) = self.workspace.tabs.get(index) else {
            return;
        };
        let title = tab.title.clone();
        let custom_title = tab.custom_title.clone();
        let Some(first) = self.tab_first_session(index) else {
            return;
        };
        let Some(session) = self.sessions.get(&first) else {
            return;
        };
        let launch = self.reopen_launch(session);
        self.closed_tabs.push(ClosedTab {
            title,
            custom_title,
            cwd: session.cwd.clone(),
            launch,
        });
        // Keep only the most recent entries; drop the oldest past the cap.
        if self.closed_tabs.len() > MAX_CLOSED_TABS {
            self.closed_tabs.remove(0);
        }
    }

    /// How a closed Claude tab comes back: resuming the conversation it held
    /// last, re-key included, when the scan has its transcript; otherwise as it
    /// was launched. A fresh tab whose transcript was never written then
    /// starts a new conversation, since there is nothing to resume.
    fn reopen_launch(&self, session: &LiveSession) -> Launch {
        if let Launch::Claude(_) = session.launch
            && let Some(id) = session.claude_session_id()
            && self.record_for(id).is_some()
        {
            return Launch::Claude(ClaudeLaunch::Resume(id.to_owned()));
        }
        session.launch.clone()
    }

    /// Reopen the most recently closed tab, relaunching it in the mode and
    /// directory it was closed in. Re-closing then reopening walks the stack in
    /// LIFO order. No effects when the stack is empty.
    pub(super) fn reopen_closed_tab(&mut self, fresh_claude_id: String) -> Vec<Effect> {
        let Some(closed) = self.closed_tabs.pop() else {
            return Vec::new();
        };
        let custom_title = closed.custom_title;
        let effects = self.launch(LaunchSpec {
            cwd: closed.cwd,
            launch: closed.launch.with_fresh_id(|| fresh_claude_id),
            title: closed.title,
        });
        // Restore the manual name on top of the derived title. `launch` opens
        // the reopened tab as the new active one, so its index is `active` — but
        // only when the launch actually opened a tab (empty effects = id
        // overflow, no tab), or we would rename an unrelated tab.
        if !effects.is_empty()
            && let Some(name) = custom_title
        {
            self.workspace.rename_tab(self.workspace.active, &name);
        }
        effects
    }

    /// The tab title for a new session (FR4): the scanned digest name for a
    /// resumed Claude session — current Claude renders status in-band and
    /// reports only its own product name as an OSC title, which the decoder
    /// discards as naming the program rather than the session, so without this
    /// every resumed tab in a repo would read alike — else the project label.
    /// A fresh or unscanned session keeps the project label; an OSC title
    /// still wins later. The kind is not part of the title: the tab chip shows
    /// it from [`App::tab_kind`], so no retitle or rename can lose it.
    #[must_use]
    pub fn tab_title(&self, cwd: &str, launch: &Launch) -> String {
        launch
            .claude_id()
            .and_then(|claude_id| self.record_for(claude_id))
            .map(|record| self.session_title(record))
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| project_label(cwd).to_owned())
    }

    /// The browsed record for the tab at `index` — the sidebar entry for its
    /// Claude conversation, so a tab hover can show the same session card.
    /// `None` for an out-of-range index, a tab with no Claude session id, or
    /// one the last scan has not found yet.
    #[must_use]
    pub fn tab_record(&self, index: usize) -> Option<&SessionRecord> {
        self.session_record(self.tab_first_session(index)?)
    }

    /// The first pane of the tab at `index` — the one a tab is named after.
    #[must_use]
    pub fn tab_first_session(&self, index: usize) -> Option<SessionId> {
        self.workspace.tabs.get(index)?.sessions().first().copied()
    }

    /// The Claude session id of the tab at `index`: its first pane's, as
    /// [`LiveSession::claude_session_id`] decides it. A tab is named after its
    /// first pane, so that pane's conversation is the one the tab stands for.
    #[must_use]
    pub fn tab_claude_session_id(&self, index: usize) -> Option<&str> {
        self.claude_session_id(self.tab_first_session(index)?)
    }

    /// The activity status to badge on the tab at `index` (FR8): the most
    /// urgent status among the sessions it hosts, or `None` for an unknown
    /// index or a tab whose sessions are no longer live.
    #[must_use]
    pub fn tab_status(&self, index: usize) -> Option<SessionStatus> {
        let tab = self.workspace.tabs.get(index)?;
        tab.sessions()
            .into_iter()
            .filter_map(|id| self.sessions.get(&id).map(|s| s.status))
            .max_by_key(|status| status.urgency())
    }

    /// The kind of program the tab at `index` runs, read from its focused
    /// pane's launch so a split mixing kinds shows the one being worked in.
    #[must_use]
    pub fn tab_kind(&self, index: usize) -> Option<SessionKind> {
        let focused = self.tab_focused_session(index)?;
        self.sessions.get(&focused).map(|s| s.launch.kind())
    }

    /// The focused pane of the tab at `index` — the one whose kind and colour
    /// the tab shows.
    #[must_use]
    pub fn tab_focused_session(&self, index: usize) -> Option<SessionId> {
        self.workspace.tabs.get(index)?.focused_session()
    }

    /// Who keeps the colour of the live pane `session`: Claude for a pane
    /// launched as Claude, termherd for a shell. Decided by the launch, as
    /// whether a Claude command may be typed is, so a pane is never coloured
    /// one way and recoloured the other.
    #[must_use]
    pub fn color_keeper(&self, session: SessionId) -> Option<ColorKeeper> {
        Some(match self.sessions.get(&session)?.launch {
            Launch::Claude(_) => ColorKeeper::Claude,
            Launch::Shell => ColorKeeper::Termherd,
        })
    }

    /// The colour the live pane `session` wears. A Claude pane wears the one
    /// `/color` set, as the last scan read it from the transcript; a shell
    /// pane wears its tab's picked colour, else that of a Claude run in it.
    /// `None` when there is none.
    #[must_use]
    pub fn session_color(&self, session: SessionId) -> Option<ClaudeColor> {
        let transcript = || self.session_record(session)?.digest.agent_color;
        let color = match self.color_keeper(session)? {
            ColorKeeper::Claude => transcript(),
            ColorKeeper::Termherd => self
                .workspace
                .tab_of(session)
                .and_then(|index| self.workspace.tabs.get(index)?.color)
                .or_else(transcript),
        };
        color.filter(|color| *color != ClaudeColor::Default)
    }

    /// Store `color` on the tab at `index` if termherd keeps its focused
    /// pane's colour; a Claude pane's is Claude's alone. A pure recolour: no
    /// effect either way.
    pub(super) fn set_tab_color(&mut self, index: usize, color: ClaudeColor) -> Vec<Effect> {
        let keeper = self
            .tab_focused_session(index)
            .and_then(|focused| self.color_keeper(focused));
        if keeper == Some(ColorKeeper::Termherd)
            && let Some(tab) = self.workspace.tabs.get_mut(index)
        {
            tab.color = Some(color);
        }
        Vec::new()
    }

    /// The colour the tab at `index` wears: its focused pane's, so a split
    /// shows the colour of the conversation being worked in — the rule
    /// [`Self::tab_kind`] follows.
    #[must_use]
    pub fn tab_color(&self, index: usize) -> Option<ClaudeColor> {
        self.session_color(self.tab_focused_session(index)?)
    }

    /// Count of sessions whose PTY is still running — the ones a quit would
    /// hard-kill. Exited sessions linger in the registry but cost nothing to
    /// drop; the count behind the quit-confirmation modal's summary line.
    #[must_use]
    pub fn live_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.status != SessionStatus::Exited)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;
    use crate::workspace::SplitDir;

    #[test]
    fn activate_tab_brings_an_earlier_session_to_focus() {
        let mut app = App::new();
        let first = launch(&mut app, "a");
        let _second = launch(&mut app, "b");
        assert_eq!(app.workspace.focused_session(), Some(_second));

        let effects = app.apply(Event::ActivateTab(0));
        assert!(effects.is_empty());
        assert_eq!(app.workspace.focused_session(), Some(first));
    }

    #[test]
    fn activate_tab_out_of_range_leaves_the_active_tab_untouched() {
        // Regression guard for the number-row jump: pressing ⌘5
        // with only two tabs open resolves to an out-of-range index, which
        // must be a silent no-op rather than a panic or a focus change.
        let mut app = App::new();
        let _first = launch(&mut app, "a");
        let second = launch(&mut app, "b");
        assert_eq!(app.workspace.active, 1);

        let effects = app.apply(Event::ActivateTab(4));
        assert!(effects.is_empty());
        assert_eq!(app.workspace.active, 1);
        assert_eq!(app.workspace.focused_session(), Some(second));
    }

    #[test]
    fn close_tab_kills_its_session_and_drops_it_from_the_registry() {
        let mut app = App::new();
        let first = launch(&mut app, "a");
        let second = launch(&mut app, "b");

        let effects = app.apply(Event::CloseTab(1));
        assert!(matches!(effects.as_slice(), [Effect::Kill(id)] if *id == second));
        assert_eq!(app.workspace.tabs.len(), 1);
        assert!(!app.sessions.contains_key(&second));
        // The surviving session stays live and focused.
        assert_eq!(app.workspace.focused_session(), Some(first));
        assert!(app.sessions.contains_key(&first));
    }

    #[test]
    fn reopen_restores_a_closed_tab_in_its_mode_and_directory() {
        // Closing a Claude tab then reopening relaunches the same kind in
        // the same directory, with its label.
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/repo".into()),
            launch: Launch::Claude(ClaudeLaunch::Resume("abc".into())),
            title: "repo".into(),
        }));
        let original = app.workspace.focused_session().expect("focused");
        app.apply(Event::CloseTab(0));
        assert!(app.workspace.tabs.is_empty());

        let effects = app.apply(Event::ReopenClosedTab {
            fresh_claude_id: "minted".into(),
        });
        let spec = match effects.as_slice() {
            [Effect::Spawn(spec)] => spec,
            other => panic!("expected one Spawn, got {other:?}"),
        };
        assert_ne!(spec.session, original, "reopen mints a fresh session id");
        assert_eq!(spec.cwd.as_deref(), Some("/repo"));
        assert_eq!(
            spec.launch,
            Launch::Claude(ClaudeLaunch::Resume("abc".into()))
        );
        assert_eq!(app.workspace.tabs.len(), 1);
        assert_eq!(app.workspace.tabs[0].title, "repo");
    }

    #[test]
    fn reopening_a_renamed_tab_restores_the_custom_title() {
        let mut app = App::new();
        launch(&mut app, "derived");
        app.apply(Event::RenameTab {
            index: 0,
            title: "Prod deploy".into(),
        });
        app.apply(Event::CloseTab(0));

        let effects = app.apply(Event::ReopenClosedTab {
            fresh_claude_id: "minted".into(),
        });
        let new_id = match effects.as_slice() {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected one Spawn, got {other:?}"),
        };
        // The manual name round-trips the close/reopen, laid back over the
        // derived title — not lost, and still a real override.
        assert_eq!(app.workspace.tabs[0].display_title(), "Prod deploy");
        assert_eq!(app.workspace.tabs[0].title, "derived");
        // Being a real override, a later relabel still cannot clobber it.
        app.apply(Event::SessionTitleChanged {
            session: new_id,
            title: "new derived".into(),
        });
        assert_eq!(app.workspace.tabs[0].display_title(), "Prod deploy");
    }

    #[test]
    fn reopen_with_nothing_closed_is_a_noop() {
        let mut app = App::new();
        assert!(
            app.apply(Event::ReopenClosedTab {
                fresh_claude_id: "minted".into(),
            })
            .is_empty()
        );
        // Even after a launch with no close, there is nothing on the stack.
        launch(&mut app, "a");
        assert!(
            app.apply(Event::ReopenClosedTab {
                fresh_claude_id: "minted".into(),
            })
            .is_empty()
        );
    }

    #[test]
    fn reopen_walks_the_close_stack_in_lifo_order() {
        // Closing A then B and reopening twice restores B first, then A.
        let mut app = App::new();
        let open = |app: &mut App, dir: &str| {
            app.apply(Event::LaunchSession(LaunchSpec {
                cwd: Some(dir.into()),
                launch: Launch::Shell,
                title: dir.into(),
            }));
        };
        open(&mut app, "/a");
        open(&mut app, "/b");
        // Close the later tab (index 1 = /b) then the remaining one (/a).
        app.apply(Event::CloseTab(1));
        app.apply(Event::CloseTab(0));
        assert!(app.workspace.tabs.is_empty());

        let first = app.apply(Event::ReopenClosedTab {
            fresh_claude_id: "minted".into(),
        });
        let second = app.apply(Event::ReopenClosedTab {
            fresh_claude_id: "minted".into(),
        });
        let cwd_of = |effects: &[Effect]| match effects {
            [Effect::Spawn(spec)] => spec.cwd.clone(),
            other => panic!("expected one Spawn, got {other:?}"),
        };
        // LIFO: the last close (/a) comes back first, then /b.
        assert_eq!(cwd_of(&first).as_deref(), Some("/a"));
        assert_eq!(cwd_of(&second).as_deref(), Some("/b"));
        // Stack drained.
        assert!(
            app.apply(Event::ReopenClosedTab {
                fresh_claude_id: "minted".into(),
            })
            .is_empty()
        );
    }

    #[test]
    fn session_title_changed_relabels_the_tab() {
        let mut app = App::new();
        let id = launch(&mut app, "old");
        let effects = app.apply(Event::SessionTitleChanged {
            session: id,
            title: "Claude's title".into(),
        });
        assert!(effects.is_empty());
        assert_eq!(app.workspace.tabs[0].title, "Claude's title");
    }

    #[test]
    fn tab_kind_follows_the_focused_pane() {
        let mut app = App::new();
        launch_claude(&mut app);
        // A split opens a shell beside the Claude pane and focuses it.
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        assert_eq!(app.tab_kind(0), Some(SessionKind::Shell));
        app.apply(Event::FocusPrevPane);
        assert_eq!(app.tab_kind(0), Some(SessionKind::Claude));
        assert_eq!(app.tab_kind(1), None, "no such tab");
    }

    #[test]
    fn tab_title_prefers_the_scanned_digest_name() {
        // The kind is shown beside the title, never written into it, so a
        // shell and a fresh Claude session in one project share the label.
        let mut app = App::new();
        assert_eq!(app.tab_title("/home/me/proj", &Launch::Shell), "proj");
        assert_eq!(
            app.tab_title("/home/me/proj", &Launch::Claude(ClaudeLaunch::Fresh(None))),
            "proj"
        );

        // Resuming a *scanned* session takes its digest name, so two
        // resumed tabs in one repo don't read alike.
        app.apply(Event::ScanCompleted(vec![record(
            "abc-123",
            "/home/me/proj",
            "fix the login bug",
        )]));
        assert_eq!(
            app.tab_title(
                "/home/me/proj",
                &Launch::Claude(ClaudeLaunch::Resume("abc-123".into())),
            ),
            "fix the login bug"
        );

        // Resuming an *unscanned* session falls back to the project label.
        assert_eq!(
            app.tab_title(
                "/home/me/proj",
                &Launch::Claude(ClaudeLaunch::Resume("not-scanned".into())),
            ),
            "proj"
        );
    }

    #[test]
    fn tab_record_resolves_a_resumed_tab_and_skips_shells_and_unknowns() {
        // A tab resuming a scanned session maps back to its record; a shell
        // tab (no resume id) and an out-of-range index map to nothing.
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record(
            "abc-123",
            "/proj",
            "fix the login bug",
        )]));
        // Tab 0: a resumed Claude session that the scan knows.
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Claude(ClaudeLaunch::Resume("abc-123".into())),
            title: "proj".into(),
        }));
        // Tab 1: a plain shell — no resume id, so no record.
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Shell,
            title: "proj".into(),
        }));
        assert_eq!(
            app.tab_record(0).map(|r| r.session_id.as_str()),
            Some("abc-123")
        );
        assert!(app.tab_record(1).is_none(), "a shell tab has no record");
        assert!(app.tab_record(9).is_none(), "an out-of-range index is None");
    }

    #[test]
    fn tab_record_resolves_a_fresh_tab_by_its_minted_id() {
        let minted = "0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f9012";
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Claude(ClaudeLaunch::Fresh(Some(minted.into()))),
            title: "proj".into(),
        }));
        assert!(app.tab_record(0).is_none(), "nothing scanned yet");
        app.apply(Event::ScanCompleted(vec![record(
            minted,
            "/proj",
            "first prompt",
        )]));
        assert_eq!(
            app.tab_record(0).map(|r| r.session_id.as_str()),
            Some(minted)
        );
    }

    #[test]
    fn reopening_a_claude_tab_resumes_the_conversation_it_held_last() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/repo".into()),
            launch: Launch::Claude(ClaudeLaunch::Resume("before".into())),
            title: "repo".into(),
        }));
        let id = app.workspace.focused_session().expect("focused");
        let started = Some("Wed Oct  7 06:48:07 2026".to_owned());
        app.apply(Event::ForegroundJobChanged {
            session: id,
            job: Some(ForegroundJob {
                pid: 42,
                started: started.clone(),
            }),
        });
        app.apply(Event::SessionFileRead {
            session: id,
            file: Some(termherd_claude::session_file::SessionFile {
                pid: 42,
                name: None,
                session_id: Some("after".into()),
                proc_start: started,
            }),
        });
        app.apply(Event::ScanCompleted(vec![record("after", "/repo", "x")]));
        app.apply(Event::CloseTab(0));
        let effects = app.apply(Event::ReopenClosedTab {
            fresh_claude_id: "unused".into(),
        });
        let [Effect::Spawn(spec)] = effects.as_slice() else {
            panic!("expected one Spawn, got {effects:?}");
        };
        assert_eq!(
            spec.launch,
            Launch::Claude(ClaudeLaunch::Resume("after".into())),
            "the re-keyed conversation, whose transcript the scan has"
        );
    }

    #[test]
    fn reopening_a_fresh_claude_tab_starts_a_new_conversation_under_the_given_id() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/repo".into()),
            launch: Launch::Claude(ClaudeLaunch::Fresh(Some("first".into()))),
            title: "repo".into(),
        }));
        app.apply(Event::CloseTab(0));
        let effects = app.apply(Event::ReopenClosedTab {
            fresh_claude_id: "second".into(),
        });
        let [Effect::Spawn(spec)] = effects.as_slice() else {
            panic!("expected one Spawn, got {effects:?}");
        };
        assert_eq!(
            spec.launch,
            Launch::Claude(ClaudeLaunch::Fresh(Some("second".into()))),
            "the closed tab's id already names a transcript"
        );
        assert_eq!(app.tab_claude_session_id(0), Some("second"));
    }

    /// A scanned record for `id` whose transcript last set `color`.
    fn coloured(id: &str, color: Option<ClaudeColor>) -> SessionRecord {
        let mut r = record(id, "/proj", "prompt");
        r.digest.agent_color = color;
        r
    }

    fn resume(app: &mut App, id: &str) -> SessionId {
        match app
            .apply(Event::LaunchSession(LaunchSpec {
                cwd: Some("/proj".into()),
                launch: Launch::Claude(ClaudeLaunch::Resume(id.into())),
                title: "proj".into(),
            }))
            .as_slice()
        {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn a_claude_tab_wears_the_colour_its_transcript_set() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![coloured(
            "abc",
            Some(ClaudeColor::Green),
        )]));
        let pane = resume(&mut app, "abc");
        assert_eq!(app.session_color(pane), Some(ClaudeColor::Green));
        assert_eq!(app.tab_color(0), Some(ClaudeColor::Green));
    }

    #[test]
    fn a_rescan_recolours_an_open_tab() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![coloured("abc", None)]));
        resume(&mut app, "abc");
        assert_eq!(app.tab_color(0), None, "not coloured yet");
        app.apply(Event::ScanCompleted(vec![coloured(
            "abc",
            Some(ClaudeColor::Purple),
        )]));
        assert_eq!(app.tab_color(0), Some(ClaudeColor::Purple));
        app.apply(Event::ScanCompleted(vec![coloured("abc", None)]));
        assert_eq!(app.tab_color(0), None, "/color default clears it");
    }

    #[test]
    fn a_fresh_tab_is_coloured_through_its_minted_id() {
        let minted = "0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f9012";
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Claude(ClaudeLaunch::Fresh(Some(minted.into()))),
            title: "proj".into(),
        }));
        app.apply(Event::ScanCompleted(vec![coloured(
            minted,
            Some(ClaudeColor::Cyan),
        )]));
        assert_eq!(app.tab_color(0), Some(ClaudeColor::Cyan));
    }

    #[test]
    fn the_focused_pane_decides_a_split_tabs_colour() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![coloured(
            "abc",
            Some(ClaudeColor::Red),
        )]));
        resume(&mut app, "abc");
        // A split opens a shell beside the Claude pane and focuses it.
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        assert_eq!(app.tab_color(0), None, "the focused shell has no colour");
        app.apply(Event::FocusPrevPane);
        assert_eq!(app.tab_color(0), Some(ClaudeColor::Red));
    }

    #[test]
    fn a_split_tabs_colour_is_its_focused_panes_even_when_its_record_is_coloured() {
        // The hover card describes the first pane's record but must name the
        // colour the outline shows, which is the focused pane's.
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![coloured(
            "abc",
            Some(ClaudeColor::Red),
        )]));
        let first = resume(&mut app, "abc");
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        assert_eq!(app.tab_first_session(0), Some(first));
        assert_eq!(
            app.tab_record(0).and_then(|r| r.digest.agent_color),
            Some(ClaudeColor::Red)
        );
        assert_eq!(app.tab_color(0), None);
    }

    #[test]
    fn a_shell_an_unknown_pane_and_an_unknown_tab_have_no_colour() {
        let mut app = App::new();
        let shell = launch(&mut app, "sh");
        assert_eq!(app.session_color(shell), None);
        assert_eq!(app.session_color(sid(99)), None);
        assert_eq!(app.tab_color(0), None);
        assert_eq!(app.tab_color(9), None);
    }

    fn pick(app: &mut App, index: usize, color: ClaudeColor) {
        let effects = app.apply(Event::SetTabColor { index, color });
        assert!(effects.is_empty(), "a pure recolour touches no PTY");
    }

    #[test]
    fn claude_keeps_a_claude_panes_colour_and_termherd_a_shells() {
        let mut app = App::new();
        let claude = launch_claude(&mut app);
        let shell = launch(&mut app, "sh");
        assert_eq!(app.color_keeper(claude), Some(ColorKeeper::Claude));
        assert_eq!(app.color_keeper(shell), Some(ColorKeeper::Termherd));
        assert_eq!(app.color_keeper(sid(99)), None);
    }

    #[test]
    fn a_shell_tab_wears_the_colour_picked_for_it_until_default_clears_it() {
        let mut app = App::new();
        let shell = launch(&mut app, "sh");
        pick(&mut app, 0, ClaudeColor::Green);
        assert_eq!(app.tab_color(0), Some(ClaudeColor::Green));
        assert_eq!(app.session_color(shell), Some(ClaudeColor::Green));
        pick(&mut app, 0, ClaudeColor::Default);
        assert_eq!(app.tab_color(0), None);
        assert_eq!(app.session_color(shell), None);
    }

    #[test]
    fn a_claude_tab_is_never_given_a_colour_of_termherds_own() {
        // Claude keeps it: a local copy would be a second truth that the next
        // `/color` typed in the session contradicts.
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![coloured(
            "abc",
            Some(ClaudeColor::Red),
        )]));
        resume(&mut app, "abc");
        pick(&mut app, 0, ClaudeColor::Blue);
        assert_eq!(app.tab_color(0), Some(ClaudeColor::Red));
        assert_eq!(app.workspace.tabs[0].color, None, "nothing stored");
    }

    #[test]
    fn a_picked_colour_beats_the_transcript_of_a_claude_run_from_a_shell() {
        // The shell tab is termherd's to colour even when a Claude runs in it,
        // and picking "none" must clear it rather than reveal Claude's.
        let mut app = App::new();
        let shell = launch(&mut app, "sh");
        let started = Some("Wed Oct  7 06:48:07 2026".to_owned());
        app.apply(Event::ForegroundJobChanged {
            session: shell,
            job: Some(ForegroundJob {
                pid: 42,
                started: started.clone(),
            }),
        });
        app.apply(Event::SessionFileRead {
            session: shell,
            file: Some(termherd_claude::session_file::SessionFile {
                pid: 42,
                name: None,
                session_id: Some("inner".into()),
                proc_start: started,
            }),
        });
        app.apply(Event::ScanCompleted(vec![coloured(
            "inner",
            Some(ClaudeColor::Pink),
        )]));
        assert_eq!(
            app.tab_color(0),
            Some(ClaudeColor::Pink),
            "Claude's, unpicked"
        );
        pick(&mut app, 0, ClaudeColor::Cyan);
        assert_eq!(app.tab_color(0), Some(ClaudeColor::Cyan));
        pick(&mut app, 0, ClaudeColor::Default);
        assert_eq!(app.tab_color(0), None);
    }

    #[test]
    fn the_focused_pane_decides_between_a_shell_tabs_colour_and_claudes() {
        let mut app = App::new();
        launch_claude(&mut app);
        // A split opens a shell beside the Claude pane and focuses it.
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        pick(&mut app, 0, ClaudeColor::Yellow);
        assert_eq!(app.tab_color(0), Some(ClaudeColor::Yellow));
        app.apply(Event::FocusPrevPane);
        assert_eq!(
            app.tab_color(0),
            None,
            "the focused Claude answers for itself"
        );
        pick(&mut app, 0, ClaudeColor::Red);
        app.apply(Event::FocusNextPane);
        assert_eq!(
            app.tab_color(0),
            Some(ClaudeColor::Yellow),
            "a pick aimed at the Claude left the shell's colour alone"
        );
    }

    #[test]
    fn a_shell_tabs_colour_moves_with_it_and_is_lost_when_it_closes() {
        let mut app = App::new();
        launch(&mut app, "a");
        launch(&mut app, "b");
        pick(&mut app, 0, ClaudeColor::Orange);
        app.apply(Event::MoveTab { from: 0, to: 1 });
        assert_eq!(app.tab_color(1), Some(ClaudeColor::Orange));
        assert_eq!(app.tab_color(0), None);

        app.apply(Event::CloseTab(1));
        app.apply(Event::ReopenClosedTab {
            fresh_claude_id: "unused".into(),
        });
        assert_eq!(app.tab_color(1), None, "a reopened tab starts uncoloured");
    }

    #[test]
    fn picking_a_colour_for_a_tab_that_is_not_there_changes_nothing() {
        let mut app = App::new();
        launch(&mut app, "sh");
        pick(&mut app, 4, ClaudeColor::Red);
        assert_eq!(app.tab_color(0), None);
    }

    #[test]
    fn tab_status_reports_the_most_urgent_session_status() {
        let mut app = App::new();
        let id = launch(&mut app, "a");
        assert_eq!(app.tab_status(0), Some(SessionStatus::Starting));

        app.apply(Event::StatusChanged {
            session: id,
            status: SessionStatus::Attention,
        });
        assert_eq!(app.tab_status(0), Some(SessionStatus::Attention));
        // Unknown tab index has no status.
        assert_eq!(app.tab_status(7), None);
    }

    #[test]
    fn live_session_count_excludes_exited_sessions() {
        // The quit-confirm summary counts sessions a quit would hard-kill:
        // everything not yet Exited, whatever its running state.
        let mut app = App::new();
        assert_eq!(app.live_session_count(), 0, "an empty app has none live");

        let a = launch(&mut app, "a");
        launch(&mut app, "b");
        assert_eq!(app.live_session_count(), 2, "two launched shells are live");

        // An idle (but not exited) session still counts — it has a process.
        app.apply(Event::StatusChanged {
            session: a,
            status: SessionStatus::Idle,
        });
        assert_eq!(app.live_session_count(), 2, "idle is still live");

        // Exiting one drops it from the count; the map may still hold it. A
        // dirty exit marks the session Exited without auto-closing its tab.
        app.apply(Event::PtyExited {
            session: a,
            clean: false,
        });
        assert_eq!(
            app.live_session_count(),
            1,
            "an exited session no longer counts"
        );
    }
}
