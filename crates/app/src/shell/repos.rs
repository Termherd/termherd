//! Declaring a repo into the sidebar (`F-repo-add`). The native picker and the
//! window file-drop both land here, so the normalisation that keeps a
//! declaration and a discovery of one repository on a single sidebar key has
//! exactly one caller — and one place to test.
//!
//! Applying the event and saying so in the log are the same pair of functions
//! for every gesture, MCP included: a second `info!` elsewhere would be a
//! second wording to keep true.

use std::path::Path;

use iced::Task;

use super::{Message, Shell};

/// Which gesture asked for the change.
///
/// The sidebar cannot answer this afterwards — a declaration made by a drop is
/// byte-for-byte the one made by the picker — so the only record is the log
/// line, and without this field a `tracing` run cannot tell which surface a
/// user actually reached.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RepoGesture {
    /// The native folder dialog behind `+ Add a repo`.
    Picker,
    /// A folder dropped on the window.
    Drop,
    /// The `✕` on an added row.
    Button,
    /// The `add_repo` / `forget_repo` tools.
    Mcp,
}

/// Rendered rather than debug-formatted, so the field reads `via=drop` beside
/// an unquoted `repo=/…`: two spellings in one line invite a grep that matches
/// one of them.
impl std::fmt::Display for RepoGesture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Picker => "picker",
            Self::Drop => "drop",
            Self::Button => "button",
            Self::Mcp => "mcp",
        })
    }
}

impl Shell {
    /// Normalise a picked folder and declare it. `None` is a cancelled dialog.
    ///
    /// The work is a handful of `stat`s, so it runs here rather than in a
    /// `Task`: nothing is gained by deferring it, and a future is where the
    /// runtime traps live.
    pub(super) fn declare_repo(
        &mut self,
        picked: Option<&Path>,
        via: RepoGesture,
    ) -> Task<Message> {
        let Some(picked) = picked else {
            return Task::none();
        };
        let Some(path) = termherd_scan::normalize_repo_path(picked) else {
            // A refusal is a dead end for the user, so it is said out loud
            // rather than swallowed (Q5): a vanished drop is indistinguishable
            // from a broken button.
            tracing::warn!(
                picked = %picked.display(),
                via = %via,
                "cannot add to the sidebar: the path does not exist"
            );
            return Task::none();
        };
        self.declare_repo_key(&path.display().to_string(), via)
    }

    /// Declare an already-normalised key. The MCP path arrives here, having
    /// normalised first because it answers with the key it kept.
    pub(super) fn declare_repo_key(&mut self, key: &str, via: RepoGesture) -> Task<Message> {
        tracing::info!(repo = %key, via = %via, "repo declared in the sidebar");
        let effects = self
            .core
            .apply(termherd_core::Event::DeclareRepo(key.to_owned()));
        Task::batch([self.perform(effects), self.refresh_docs()])
    }

    /// Drop a declaration. The row may survive on its sessions, which is worth
    /// logging: "forgotten" and "gone from the sidebar" are not the same event,
    /// and a reader of the log would otherwise assume the second.
    pub(super) fn forget_repo_key(&mut self, key: &str, via: RepoGesture) -> Task<Message> {
        let effects = self
            .core
            .apply(termherd_core::Event::ForgetRepo(key.to_owned()));
        tracing::info!(
            repo = %key,
            via = %via,
            kept = self.core.repo_has_sessions(key),
            "repo declaration dropped"
        );
        Task::batch([self.perform(effects), self.refresh_docs()])
    }
}
