//! The plan / memory document open in the main pane (F-plans-memory): the
//! editor's view-state struct and the off-thread save. Split from the shell's
//! state machine so the editor buffer and its save/concurrency guard live in
//! one place. The document *I/O* (read/write/scope) lives in [`crate::docs`];
//! this module holds only the open-in-pane state.

use std::path::PathBuf;
use std::time::SystemTime;

use iced::Task;
use iced::widget::text_editor;

use super::routing::KeyboardOwner;
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

/// Unsaved edits about to be thrown away, held until the user answers. Each
/// way out of a modified doc arms this instead of discarding outright.
pub(super) enum DocDiscard {
    /// The editor is closing (its button or `escape`).
    Close,
    /// Another doc was clicked in the sidebar. It is armed by the click, a
    /// deliberate gesture, and read only once the edits are let go: arming on
    /// the read's answer instead would raise the prompt mid-typing, where the
    /// next Enter meant as a newline would answer it.
    Open { label: String, path: PathBuf },
}

impl OpenDoc {
    pub(super) fn loaded(
        label: String,
        path: PathBuf,
        content: &str,
        mtime: Option<SystemTime>,
    ) -> Self {
        let writable = crate::docs::is_writable(&path);
        Self {
            label,
            path,
            content: text_editor::Content::with_text(content),
            loaded_mtime: mtime,
            writable,
            dirty: false,
            feedback: None,
        }
    }
}

/// The outcome of the last save attempt, surfaced in the editor header.
pub(super) enum DocFeedback {
    Saved,
    Error(String),
}

impl Shell {
    /// Read a doc off the UI thread; it opens when [`Message::DocLoaded`]
    /// answers.
    pub(super) fn read_doc(label: String, path: PathBuf) -> Task<Message> {
        let read_path = path.clone();
        Task::perform(
            async move {
                let content =
                    crate::docs::read(&read_path).unwrap_or_else(crate::strings::doc_read_failed);
                let mtime = crate::docs::mtime(&read_path).ok();
                (content, mtime)
            },
            move |(content, mtime)| Message::DocLoaded {
                label: label.clone(),
                path: path.clone(),
                content,
                mtime,
            },
        )
    }

    pub(super) fn request_doc(&mut self, label: String, path: PathBuf) -> Task<Message> {
        if self.has_unsaved_doc() {
            self.discarding_doc = Some(DocDiscard::Open { label, path });
            return Task::none();
        }
        Self::read_doc(label, path)
    }

    /// Every read is asked for with no unsaved edits, so any found when it
    /// answers were typed while it was in flight; the answer is dropped rather
    /// than allowed to take them.
    pub(super) fn show_doc(&mut self, doc: OpenDoc) -> Task<Message> {
        if self.has_unsaved_doc() {
            tracing::info!(
                path = %doc.path.display(),
                "doc load dropped: the open doc was edited while it was read"
            );
            return Task::none();
        }
        self.open_doc = Some(doc);
        Task::none()
    }

    /// Apply one editor action; only a real edit marks the doc modified, so
    /// moving the cursor or selecting leaves it clean.
    pub(super) fn edit_doc(&mut self, action: text_editor::Action) -> Task<Message> {
        if !self.doc_accepts_edits() {
            return Task::none();
        }
        if let Some(doc) = &mut self.open_doc {
            let edits = action.is_edit();
            doc.content.perform(action);
            if edits {
                doc.dirty = true;
                doc.feedback = None;
            }
        }
        Task::none()
    }

    /// Record how a save ended: a success re-bases the concurrency guard and
    /// clears the modified mark, a refusal is shown and keeps the edits.
    pub(super) fn on_doc_saved(
        &mut self,
        result: Result<SystemTime, crate::docs::SaveError>,
    ) -> Task<Message> {
        if let Some(doc) = &mut self.open_doc {
            match result {
                Ok(new_mtime) => {
                    doc.loaded_mtime = Some(new_mtime);
                    doc.dirty = false;
                    doc.feedback = Some(DocFeedback::Saved);
                }
                Err(error) => {
                    doc.feedback = Some(DocFeedback::Error(error.to_string()));
                }
            }
        }
        Task::none()
    }

    pub(super) fn close_doc(&mut self) -> Task<Message> {
        if self.has_unsaved_doc() {
            self.discarding_doc = Some(DocDiscard::Close);
        } else {
            self.open_doc = None;
        }
        Task::none()
    }

    pub(super) fn confirm_doc_discard(&mut self) -> Task<Message> {
        let Some(discard) = self.discarding_doc.take() else {
            return Task::none();
        };
        self.open_doc = None;
        match discard {
            DocDiscard::Close => Task::none(),
            DocDiscard::Open { label, path } => Self::read_doc(label, path),
        }
    }

    pub(super) fn cancel_doc_discard(&mut self) -> Task<Message> {
        self.discarding_doc = None;
        Task::none()
    }

    /// The one test every way out of the editor asks before it discards.
    pub(super) fn has_unsaved_doc(&self) -> bool {
        self.open_doc.as_ref().is_some_and(|doc| doc.dirty)
    }

    /// The editor takes input only while it owns the keyboard, as the search
    /// box does: any modal over it blocks the mouse alone, so a focused editor
    /// would otherwise also take the keys that answer the modal. Read by the
    /// view and by the edit handler alike, so whichever arrives first, such a
    /// key changes nothing.
    pub(super) fn doc_accepts_edits(&self) -> bool {
        self.keyboard_owner() == Some(KeyboardOwner::Doc)
    }

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
