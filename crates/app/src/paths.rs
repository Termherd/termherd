//! Shared filesystem locations — the user's home dir, the `~/.termherd` app
//! data dir (PRD §7) and Claude's `~/.claude`. Resolved in one place so every
//! store speaks the same `USERPROFILE`/`HOME` precedence and the same dir name;
//! the alternative is the seven hand-rolled copies this replaces, which drift
//! the day the location moves.

use std::path::PathBuf;

/// The user's home directory: `%USERPROFILE%` (Windows) then `$HOME` (Unix).
/// `None` when neither is set.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// The app data dir (PRD §7) every store reads and writes under. Resolved once
/// in `main()` and injected, so a store never finds the user's files on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir(PathBuf);

impl DataDir {
    /// `~/.termherd`. `None` when no home directory is set, in which case
    /// nothing is persisted.
    #[must_use]
    pub fn user() -> Option<Self> {
        home_dir().map(|home| Self(home.join(".termherd")))
    }

    /// `<dir>/<name>`, e.g. `metadata.json`.
    #[must_use]
    pub fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

/// `~/.claude` — Claude Code's own data dir, which termherd reads (plans,
/// memory, per-process session files) and never owns. `None` when no home
/// directory is set.
#[must_use]
pub fn claude_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".claude"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_dir_is_the_dot_claude_folder_in_home() {
        assert_eq!(claude_dir(), home_dir().map(|home| home.join(".claude")));
    }
}
