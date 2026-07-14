//! Tab lifecycle: close, the reopen-closed stack, and the derived tab title /
//! status / record read models.

use crate::browser::{SessionRecord, project_label};

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
        let Some(first) = tab.sessions().first().copied() else {
            return;
        };
        let Some(session) = self.sessions.get(&first) else {
            return;
        };
        self.closed_tabs.push(ClosedTab {
            title,
            custom_title,
            cwd: session.cwd.clone(),
            launch: session.launch.clone(),
        });
        // Keep only the most recent entries; drop the oldest past the cap.
        if self.closed_tabs.len() > MAX_CLOSED_TABS {
            self.closed_tabs.remove(0);
        }
    }

    /// Reopen the most recently closed tab, relaunching it in the mode and
    /// directory it was closed in. Re-closing then reopening walks the stack in
    /// LIFO order. No effects when the stack is empty.
    pub(super) fn reopen_closed_tab(&mut self) -> Vec<Effect> {
        let Some(closed) = self.closed_tabs.pop() else {
            return Vec::new();
        };
        let custom_title = closed.custom_title;
        let effects = self.launch(LaunchSpec {
            cwd: closed.cwd,
            launch: closed.launch,
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
    /// resumed Claude session — current Claude renders status in-band and emits
    /// no OSC title, so without this every resumed tab in a repo would read
    /// alike — else the kind label `{project} {glyph}`. A fresh or unscanned
    /// session keeps the kind label; an OSC title still wins later. The kind
    /// glyphs are the caller's (view-side constants), so core carries no
    /// presentation literals.
    #[must_use]
    pub fn tab_title(
        &self,
        cwd: &str,
        launch: &Launch,
        shell_glyph: &str,
        claude_glyph: &str,
    ) -> String {
        let label = project_label(cwd);
        match launch {
            Launch::Shell => format!("{label} {shell_glyph}"),
            Launch::Claude {
                resume: Some(claude_id),
            } => self
                .record_for(claude_id)
                .map(|record| self.session_title(record))
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("{label} {claude_glyph}")),
            Launch::Claude { resume: None } => format!("{label} {claude_glyph}"),
        }
    }

    /// The browsed record for the tab at `index` — the sidebar entry its first
    /// pane resumes, so a tab hover can show the same session card. `None`
    /// for an out-of-range index, or a tab whose first pane is a shell or a
    /// fresh, not-yet-scanned session (no resume id / no record).
    #[must_use]
    pub fn tab_record(&self, index: usize) -> Option<&SessionRecord> {
        let tab = self.workspace.tabs.get(index)?;
        let first = tab.sessions().first().copied()?;
        let claude_id = self.sessions.get(&first)?.launch.resume_id()?;
        self.record_for(claude_id)
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
            launch: Launch::Claude {
                resume: Some("abc".into()),
            },
            title: "repo 🤖".into(),
        }));
        let original = app.workspace.focused_session().expect("focused");
        app.apply(Event::CloseTab(0));
        assert!(app.workspace.tabs.is_empty());

        let effects = app.apply(Event::ReopenClosedTab);
        let spec = match effects.as_slice() {
            [Effect::Spawn(spec)] => spec,
            other => panic!("expected one Spawn, got {other:?}"),
        };
        assert_ne!(spec.session, original, "reopen mints a fresh session id");
        assert_eq!(spec.cwd.as_deref(), Some("/repo"));
        assert_eq!(
            spec.launch,
            Launch::Claude {
                resume: Some("abc".into())
            }
        );
        assert_eq!(app.workspace.tabs.len(), 1);
        assert_eq!(app.workspace.tabs[0].title, "repo 🤖");
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

        let effects = app.apply(Event::ReopenClosedTab);
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
        assert!(app.apply(Event::ReopenClosedTab).is_empty());
        // Even after a launch with no close, there is nothing on the stack.
        launch(&mut app, "a");
        assert!(app.apply(Event::ReopenClosedTab).is_empty());
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

        let first = app.apply(Event::ReopenClosedTab);
        let second = app.apply(Event::ReopenClosedTab);
        let cwd_of = |effects: &[Effect]| match effects {
            [Effect::Spawn(spec)] => spec.cwd.clone(),
            other => panic!("expected one Spawn, got {other:?}"),
        };
        // LIFO: the last close (/a) comes back first, then /b.
        assert_eq!(cwd_of(&first).as_deref(), Some("/a"));
        assert_eq!(cwd_of(&second).as_deref(), Some("/b"));
        // Stack drained.
        assert!(app.apply(Event::ReopenClosedTab).is_empty());
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
    fn tab_title_prefers_the_scanned_digest_name() {
        // Glyphs are the caller's (view-side constants), passed in; core owns
        // the digest-name-else-kind-label policy.
        let mut app = App::new();
        // A shell gets the project label with the shell glyph.
        assert_eq!(
            app.tab_title("/home/me/proj", &Launch::Shell, "$", "🤖"),
            "proj $"
        );

        // A fresh Claude session (no resume) gets the Claude glyph.
        assert_eq!(
            app.tab_title("/home/me/proj", &Launch::Claude { resume: None }, "$", "🤖"),
            "proj 🤖"
        );

        // Resuming a *scanned* session takes its digest name (no glyph), so two
        // resumed tabs in one repo don't read alike.
        app.apply(Event::ScanCompleted(vec![record(
            "abc-123",
            "/home/me/proj",
            "fix the login bug",
        )]));
        assert_eq!(
            app.tab_title(
                "/home/me/proj",
                &Launch::Claude {
                    resume: Some("abc-123".into())
                },
                "$",
                "🤖",
            ),
            "fix the login bug"
        );

        // Resuming an *unscanned* session falls back to the kind label.
        assert_eq!(
            app.tab_title(
                "/home/me/proj",
                &Launch::Claude {
                    resume: Some("not-scanned".into())
                },
                "$",
                "🤖",
            ),
            "proj 🤖"
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
            launch: Launch::Claude {
                resume: Some("abc-123".into()),
            },
            title: "proj 🤖".into(),
        }));
        // Tab 1: a plain shell — no resume id, so no record.
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Shell,
            title: "proj $".into(),
        }));
        assert_eq!(
            app.tab_record(0).map(|r| r.session_id.as_str()),
            Some("abc-123")
        );
        assert!(app.tab_record(1).is_none(), "a shell tab has no record");
        assert!(app.tab_record(9).is_none(), "an out-of-range index is None");
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
