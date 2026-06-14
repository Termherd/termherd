//! Per-session user metadata (`F-session-metadata`) — a thin overlay on the
//! read-only Claude sessions under `~/.claude`. We never write there; this
//! lives in `~/.termherd`, keyed by the Claude session id. Pure data; the
//! persistence adapter in `app` serialises it.

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
