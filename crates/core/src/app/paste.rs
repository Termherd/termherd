//! Whether a paste may be written straight away, and the paste held back when
//! it may not.
//!
//! A terminal that never enabled bracketed paste cannot tell pasted text from
//! typed text, so every line break in it is an Enter: a multi-line clipboard
//! runs line by line as it arrives, including a line the user never saw on the
//! page they copied it from. Such a paste waits for the user's confirmation.
//! A bracketed target receives the text as one block, and text without a line
//! break submits nothing, so neither asks.

use super::*;

/// A paste waiting on the user's confirmation, and the session it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPaste {
    pub session: SessionId,
    pub text: String,
    /// The target's bracketed-paste mode when the paste was asked for, so the
    /// confirmed write frames the text exactly as an unguarded one would have.
    pub bracketed: bool,
}

impl PendingPaste {
    /// How many lines the paste would run, for the prompt to name.
    #[must_use]
    pub fn lines(&self) -> usize {
        line_count(&self.text)
    }
}

/// Whether pasting `text` into a terminal whose bracketed-paste mode is
/// `bracketed` must be confirmed first: the terminal would read any of its line
/// breaks — `\n`, `\r\n` or a lone `\r`, a trailing one included — as Enter.
#[must_use]
pub fn paste_needs_confirmation(text: &str, bracketed: bool) -> bool {
    !bracketed && text.contains(['\n', '\r'])
}

/// The lines in `text`, counting `\n`, `\r\n` and a lone `\r` each as one
/// break; a trailing break ends the last line rather than opening another.
#[must_use]
pub fn line_count(text: &str) -> usize {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .count()
}

impl App {
    /// Paste `text` into `session`, or hold it for confirmation when the
    /// terminal would run its lines one by one. A paste of nothing, into a
    /// session that is gone, or arriving while another waits, writes nothing:
    /// the prompt already up is the one question the keyboard is answering.
    pub(super) fn paste(
        &mut self,
        session: SessionId,
        text: String,
        bracketed: bool,
    ) -> Vec<Effect> {
        if text.is_empty() || self.pending_paste().is_some() || !self.is_live(session) {
            return Vec::new();
        }
        if paste_needs_confirmation(&text, bracketed) {
            self.pending_paste = Some(PendingPaste {
                session,
                text,
                bracketed,
            });
            return Vec::new();
        }
        vec![Effect::Paste {
            session,
            text,
            bracketed,
        }]
    }

    /// Write the paste the user confirmed, if its session is still there.
    pub(super) fn confirm_paste(&mut self) -> Vec<Effect> {
        let Some(PendingPaste {
            session,
            text,
            bracketed,
        }) = self.pending_paste.take()
        else {
            return Vec::new();
        };
        self.if_live(
            session,
            Effect::Paste {
                session,
                text,
                bracketed,
            },
        )
    }

    /// The paste waiting on the user's confirmation. One whose session has
    /// since exited or closed is no longer asked about, so the prompt cannot
    /// outlive the terminal it names.
    #[must_use]
    pub fn pending_paste(&self) -> Option<&PendingPaste> {
        self.pending_paste
            .as_ref()
            .filter(|pending| self.is_live(pending.session))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::app::testsupport::*;

    #[test]
    fn only_an_unbracketed_paste_with_a_line_break_asks() {
        let table: [(&str, bool, bool); 14] = [
            ("echo hi", false, false),
            ("echo hi", true, false),
            ("a\nb", false, true),
            ("a\nb", true, false),
            ("a\r\nb", false, true),
            ("a\r\nb", true, false),
            ("a\rb", false, true),
            ("a\rb", true, false),
            ("ls\n", false, true),
            ("ls\n", true, false),
            ("ls\r\n", false, true),
            ("\n", false, true),
            ("", false, false),
            ("tab\tand spaces  ", false, false),
        ];
        for (text, bracketed, asks) in table {
            assert_eq!(
                paste_needs_confirmation(text, bracketed),
                asks,
                "{text:?}, bracketed: {bracketed}"
            );
        }
    }

    #[test]
    fn the_line_count_names_the_lines_a_paste_would_run() {
        let table: [(&str, usize); 9] = [
            ("one", 1),
            ("ls\n", 1),
            ("ls\r\n", 1),
            ("\n", 1),
            ("a\nb", 2),
            ("a\r\nb\r\n", 2),
            ("a\rb\r", 2),
            ("a\n\nb", 3),
            ("echo safe\n rm -rf ~\n", 2),
        ];
        for (text, lines) in table {
            assert_eq!(line_count(text), lines, "{text:?}");
        }
    }

    fn paste(app: &mut App, session: SessionId, text: &str, bracketed: bool) -> Vec<Effect> {
        app.apply(Event::Paste {
            session,
            text: text.to_owned(),
            bracketed,
        })
    }

    fn pasted(effects: &[Effect]) -> Vec<(SessionId, String, bool)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Paste {
                    session,
                    text,
                    bracketed,
                } => Some((*session, text.clone(), *bracketed)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_single_line_paste_is_written_at_once() {
        let mut app = App::new();
        let session = launch(&mut app, "a");
        let effects = paste(&mut app, session, "echo hi", false);
        assert_eq!(pasted(&effects), vec![(session, "echo hi".into(), false)]);
        assert_eq!(app.pending_paste(), None);
    }

    #[test]
    fn a_bracketed_multi_line_paste_is_written_at_once() {
        let mut app = App::new();
        let session = launch(&mut app, "a");
        let effects = paste(&mut app, session, "a\nb", true);
        assert_eq!(pasted(&effects), vec![(session, "a\nb".into(), true)]);
        assert_eq!(app.pending_paste(), None);
    }

    #[test]
    fn an_unbracketed_multi_line_paste_writes_nothing_and_asks() {
        let mut app = App::new();
        let session = launch(&mut app, "a");
        let effects = paste(&mut app, session, "echo safe\nrm -rf ~\n", false);
        assert!(effects.is_empty(), "{effects:?}");
        let pending = app.pending_paste().expect("the paste waits");
        assert_eq!(pending.session, session);
        assert_eq!(pending.lines(), 2);
    }

    #[test]
    fn confirming_writes_the_held_paste_once() {
        let mut app = App::new();
        let session = launch(&mut app, "a");
        paste(&mut app, session, "a\r\nb", false);
        let effects = app.apply(Event::ConfirmPaste);
        assert_eq!(pasted(&effects), vec![(session, "a\r\nb".into(), false)]);
        assert_eq!(app.pending_paste(), None, "the prompt is answered");
        assert!(app.apply(Event::ConfirmPaste).is_empty(), "and only once");
    }

    #[test]
    fn cancelling_writes_nothing() {
        let mut app = App::new();
        let session = launch(&mut app, "a");
        paste(&mut app, session, "a\nb", false);
        assert!(app.apply(Event::CancelPaste).is_empty());
        assert_eq!(app.pending_paste(), None);
        assert!(app.apply(Event::ConfirmPaste).is_empty(), "nothing left");
    }

    #[test]
    fn a_paste_arriving_while_another_waits_is_dropped() {
        let mut app = App::new();
        let session = launch(&mut app, "a");
        paste(&mut app, session, "first\nlines", false);
        assert!(paste(&mut app, session, "single", false).is_empty());
        assert!(paste(&mut app, session, "second\nlines", false).is_empty());
        let effects = app.apply(Event::ConfirmPaste);
        assert_eq!(
            pasted(&effects),
            vec![(session, "first\nlines".into(), false)]
        );
    }

    #[test]
    fn a_paste_whose_session_exited_is_no_longer_asked_and_writes_nothing() {
        let mut app = App::new();
        let session = launch(&mut app, "a");
        paste(&mut app, session, "a\nb", false);
        app.apply(Event::PtyExited {
            session,
            clean: true,
        });
        assert_eq!(app.pending_paste(), None, "the prompt lets go");
        assert!(app.apply(Event::ConfirmPaste).is_empty());
        let other = launch(&mut app, "b");
        let effects = paste(&mut app, other, "x", false);
        assert_eq!(pasted(&effects).len(), 1, "a dead prompt blocks nothing");
    }

    #[test]
    fn a_paste_into_no_live_session_writes_nothing() {
        let mut app = App::new();
        assert!(paste(&mut app, sid(9), "x", false).is_empty());
        assert!(paste(&mut app, sid(9), "a\nb", false).is_empty());
        assert_eq!(app.pending_paste(), None);
    }

    #[test]
    fn a_paste_of_nothing_writes_nothing() {
        let mut app = App::new();
        let session = launch(&mut app, "a");
        assert!(paste(&mut app, session, "", false).is_empty());
        assert!(paste(&mut app, session, "", true).is_empty());
    }

    proptest! {
        #[test]
        fn a_single_line_or_bracketed_paste_never_asks(
            raw in "(?s)([a-z \t]|\r|\n|\r\n|.){1,40}",
            bracketed in any::<bool>(),
        ) {
            let text = if bracketed { raw } else { raw.replace(['\r', '\n'], "") };
            prop_assume!(!text.is_empty());
            let mut app = App::new();
            let session = launch(&mut app, "a");
            let effects = paste(&mut app, session, &text, bracketed);
            prop_assert_eq!(pasted(&effects), vec![(session, text, bracketed)]);
            prop_assert_eq!(app.pending_paste(), None);
        }
    }
}
