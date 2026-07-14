//! OSC 9 desktop notifications: whether a session's alert reaches the OS
//! notification centre, and with what title/body.

use super::*;

/// Shown as the desktop notification body when Claude fires a bare OSC 9 with
/// no text of its own.
const DEFAULT_NOTIFICATION_BODY: &str = "Claude needs your attention";

/// Notification title fallback when a session somehow has no hosting tab;
/// a broken invariant in practice, never the normal path.
const APP_NAME: &str = "TermHerd";

impl App {
    /// Decide whether an OSC 9 notification reaches the OS notification
    /// centre, and with what title/body. Only live sessions are worth alerting
    /// on — an unknown or exited session has nothing to return to, so it is
    /// dropped. The title is the session's tab label (what the user sees, and
    /// tracks OSC-24 renames); a blank body falls back to a default message.
    ///
    /// Also dropped: a session that is both the active tab's focused
    /// pane *and* the window has OS focus — the user is already looking at
    /// it, so no banner is needed. Any other live session still gets one,
    /// including a background tab while the window is focused: the OS's own
    /// per-window banner suppression only covers the focused-tab case, and a
    /// background tab needs the effect to reach the OS (or an in-app cue) to
    /// be seen at all.
    pub(super) fn notify_session(&self, session: SessionId, body: String) -> Vec<Effect> {
        if !self.is_live(session) {
            return Vec::new();
        }
        if self.window_focused && self.workspace.focused_session() == Some(session) {
            return Vec::new();
        }
        // A live session is always hosted by a tab, so `session_title` returns
        // `Some`; the app-name fallback only guards a broken invariant.
        let title = self
            .workspace
            .session_title(session)
            .unwrap_or(APP_NAME)
            .to_owned();
        let body = if body.trim().is_empty() {
            DEFAULT_NOTIFICATION_BODY.to_owned()
        } else {
            body
        };
        vec![Effect::Notify { title, body }]
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::*;
    use crate::app::testsupport::*;

    #[test]
    fn osc9_notification_posts_a_desktop_notification_titled_with_its_session() {
        let mut app = App::new();
        let id = launch(&mut app, "myproj");

        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "Claude needs your attention".into(),
        });

        // The body is Claude's own message; the title names which session wants
        // the user, taken from the tab the user sees.
        assert_eq!(
            notify_effect(&effects),
            Some(("myproj", "Claude needs your attention"))
        );
    }

    #[test]
    fn a_blank_notification_body_falls_back_to_a_default_message() {
        let mut app = App::new();
        let id = launch(&mut app, "myproj");

        // Claude sometimes fires a bare OSC 9 with no text; the OS notification
        // still has to say something actionable.
        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "   ".into(),
        });

        assert_eq!(
            notify_effect(&effects),
            Some(("myproj", DEFAULT_NOTIFICATION_BODY))
        );
    }

    #[test]
    fn a_notification_for_an_unknown_session_is_dropped() {
        let mut app = App::new();
        let _present = launch(&mut app, "myproj");

        let effects = app.apply(Event::SessionNotified {
            session: SessionId(NonZeroU64::new(9_999).expect("non-zero")),
            body: "ghost".into(),
        });

        assert_eq!(notify_effect(&effects), None);
    }

    #[test]
    fn a_notification_for_an_exited_session_is_dropped() {
        let mut app = App::new();
        let id = launch(&mut app, "myproj");
        app.apply(Event::PtyExited {
            session: id,
            clean: false,
        });

        // Nothing to return to — a dead session must not raise a desktop alert.
        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "too late".into(),
        });

        assert_eq!(notify_effect(&effects), None);
    }

    #[test]
    fn a_notification_follows_the_sessions_latest_tab_title() {
        let mut app = App::new();
        let id = launch(&mut app, "old name");
        // Claude relabels the tab over OSC; the notification title must
        // track that, not the launch label.
        app.apply(Event::SessionTitleChanged {
            session: id,
            title: "renamed".into(),
        });

        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "ping".into(),
        });

        assert_eq!(notify_effect(&effects), Some(("renamed", "ping")));
    }

    #[test]
    fn a_notification_for_the_viewed_session_is_dropped_while_the_window_is_focused() {
        let mut app = App::new();
        let id = launch(&mut app, "myproj");
        app.apply(Event::WindowFocusChanged(true));

        // The user is looking straight at this session; no banner is needed.
        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "ping".into(),
        });

        assert_eq!(notify_effect(&effects), None);
    }

    #[test]
    fn a_notification_for_a_background_tab_still_posts_while_the_window_is_focused() {
        let mut app = App::new();
        let background = launch(&mut app, "a");
        let _foreground = launch(&mut app, "b");
        assert_eq!(app.workspace.focused_session(), Some(_foreground));
        app.apply(Event::WindowFocusChanged(true));

        // The active tab is "b"; a notification from "a" (a background tab) must
        // still reach the OS — the OS's own per-window suppression only covers
        // the tab the user is actually viewing.
        let effects = app.apply(Event::SessionNotified {
            session: background,
            body: "ping".into(),
        });

        assert_eq!(notify_effect(&effects), Some(("a", "ping")));
    }

    #[test]
    fn a_notification_for_the_viewed_session_still_posts_while_the_window_is_unfocused() {
        let mut app = App::new();
        let id = launch(&mut app, "myproj");
        app.apply(Event::WindowFocusChanged(true));
        app.apply(Event::WindowFocusChanged(false));

        // Termherd itself is out of focus (another app is frontmost); today's
        // OS-suppression behaviour still applies, so the effect must still fire.
        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "ping".into(),
        });

        assert_eq!(notify_effect(&effects), Some(("myproj", "ping")));
    }

    proptest::proptest! {
        /// For any live session and any body, exactly one notification is
        /// posted, its title is the tab title and its body is preserved
        /// verbatim when non-blank — and `apply` never panics (Q5).
        #[test]
        fn live_session_notifications_preserve_body_and_title(
            title in "[^\u{0}]{0,40}",
            body in "\\PC{1,80}",
        ) {
            let mut app = App::new();
            let id = launch(&mut app, title.as_str());

            let effects = app.apply(Event::SessionNotified { session: id, body: body.clone() });

            let expected_body = if body.trim().is_empty() {
                DEFAULT_NOTIFICATION_BODY.to_owned()
            } else {
                body
            };
            proptest::prop_assert_eq!(
                notify_effect(&effects),
                Some((title.as_str(), expected_body.as_str()))
            );
        }

        /// A notification for a session that was never launched is always
        /// dropped, whatever the body — no panic, no effect.
        #[test]
        fn unknown_session_notifications_are_always_dropped(
            raw_id in 1u64..1_000_000,
            body in "\\PC{0,80}",
        ) {
            let mut app = App::new();
            let id = SessionId(NonZeroU64::new(raw_id).expect("non-zero"));

            let effects = app.apply(Event::SessionNotified { session: id, body });

            proptest::prop_assert_eq!(notify_effect(&effects), None);
        }
    }
}
