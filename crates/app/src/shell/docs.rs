//! The plan / memory document open in the main pane (F-plans-memory): the
//! editor's view-state struct and the off-thread save. Split from the shell's
//! state machine so the editor buffer and its save/concurrency guard live in
//! one place. The document *I/O* (read/write/scope) lives in [`crate::docs`];
//! this module holds only the open-in-pane state.

use std::path::PathBuf;
use std::time::SystemTime;

use iced::Task;
use iced::widget::text_editor;

use super::{Message, Shell};

/// A plan / memory document open in the main pane (F-plans-memory). Holds the
/// editable buffer plus the state the save path needs: where it lives, whether
/// it is in the writable scope, and the mtime captured at load for the
/// concurrency guard.
pub(super) struct OpenDoc {
    /// Sidebar label, shown in the editor header.
    pub(super) label: String,
    /// File on disk; the scope predicate and save are measured against it.
    pub(super) path: PathBuf,
    /// The editable text buffer (iced text editor state).
    pub(super) content: text_editor::Content,
    /// mtime captured at load; the baseline for the concurrent-write guard.
    /// `None` if it could not be read (then no conflict can be detected).
    pub(super) loaded_mtime: Option<SystemTime>,
    /// Whether the write-scope predicate permits saving this path.
    pub(super) writable: bool,
    /// Unsaved edits since load or the last successful save.
    pub(super) dirty: bool,
    /// Transient feedback after a save attempt.
    pub(super) feedback: Option<DocFeedback>,
}

/// The outcome of the last save attempt, surfaced in the editor header.
pub(super) enum DocFeedback {
    Saved,
    Error(String),
}

impl Shell {
    /// Save the open doc off-thread, if there is one with unsaved edits in the
    /// writable scope. A no-op otherwise, so the save chord/button is harmless
    /// when nothing needs writing.
    pub(super) fn save_open_doc(&self) -> Task<Message> {
        let Some(doc) = &self.open_doc else {
            return Task::none();
        };
        if !doc.writable || !doc.dirty {
            return Task::none();
        }
        let path = doc.path.clone();
        let contents = doc.content.text();
        let open_mtime = doc.loaded_mtime;
        Task::perform(
            async move { crate::docs::save(&path, &contents, open_mtime) },
            Message::DocSaved,
        )
    }
}
