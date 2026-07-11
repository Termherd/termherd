//! User metadata — a thin overlay on the read-only Claude sessions under
//! `~/.claude`. We never write there; this lives in `~/.termherd`. Two keyings
//! share one file: [`SessionMeta`] per Claude session id (`F-session-metadata`)
//! and [`RepoMeta`] per real project path (`F-favorites`, repo-level). Pure
//! data; the persistence adapter in `app` serialises the whole [`Overlay`].

use std::collections::HashMap;

/// Star / archive / custom-title overlay for one session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMeta {
    /// Pinned to the top of its project group.
    pub starred: bool,
    /// Hidden from the browser unless "show archived" is on.
    pub archived: bool,
    /// A user title that overrides the derived one.
    pub title: Option<String>,
}

impl SessionMeta {
    /// True when nothing is set — such entries need not be stored, so a toggle
    /// back to the defaults drops the entry instead of persisting noise.
    #[must_use]
    pub fn is_default(&self) -> bool {
        !self.starred && !self.archived && self.title.is_none()
    }
}

/// User overlay for one project/repo, keyed by its real project path
/// (`F-favorites`). A struct rather than a bare `bool` so it can grow further
/// per-repo settings (e.g. launch dirs) without a second on-disk truth.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoMeta {
    /// Pinned to the top of the sidebar.
    pub starred: bool,
}

impl RepoMeta {
    /// True when nothing is set — such entries are dropped rather than persisted
    /// as noise, mirroring [`SessionMeta::is_default`].
    #[must_use]
    pub fn is_default(&self) -> bool {
        !self.starred
    }
}

/// The whole user overlay: both keyings that share `~/.termherd/metadata.json`.
/// Carried as one unit through [`crate::Event::MetadataLoaded`] and
/// [`crate::Effect::SaveMetadata`] so a save always writes the complete file —
/// there is no partial write that could drop one map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overlay {
    /// Per Claude session id.
    pub sessions: HashMap<String, SessionMeta>,
    /// Per real project path.
    pub repos: HashMap<String, RepoMeta>,
}
