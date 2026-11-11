//! Revealing the focused session in the OS file manager: its working directory,
//! or its transcript selected. The path is resolved here, the spawn is the
//! effect executor's.

use iced::Task;
use termherd_core::{Effect, OpenTarget};

use super::{Message, Shell};

/// What a reveal shows of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reveal {
    /// The directory the session runs in, opened.
    Dir,
    /// The Claude transcript, selected in its folder.
    Transcript,
}

impl Shell {
    /// Reveal `what` of the focused session. `None` when there is nothing to
    /// show: no focused pane, a directory not known, or a transcript no scan
    /// has found.
    pub(super) fn reveal_focused(&mut self, what: Reveal) -> Option<Task<Message>> {
        let target = self.focused_reveal(what)?;
        Some(self.perform(vec![Effect::OpenPath(target)]))
    }

    /// Where revealing `what` of the focused session points.
    pub(super) fn focused_reveal(&mut self, what: Reveal) -> Option<OpenTarget> {
        let session = self.core.workspace.focused_session()?;
        Some(match what {
            Reveal::Dir => OpenTarget::SystemHandler {
                path: self.core.sessions.get(&session)?.cwd.clone()?.into(),
            },
            Reveal::Transcript => {
                // The id a re-keyed conversation (`/clear`) now writes under is
                // only as fresh as the last read of its session file.
                self.refresh_session_file(session);
                OpenTarget::Reveal {
                    path: self.core.transcript_of(session)?.to_owned(),
                }
            }
        })
    }
}
