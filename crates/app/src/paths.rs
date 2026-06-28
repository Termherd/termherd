//! Shared filesystem locations — the user's home dir and the `~/.termherd` app
//! data dir (PRD §7). Resolved in one place so every store speaks the same
//! `USERPROFILE`/`HOME` precedence and the same dir name; the alternative is the
//! seven hand-rolled copies this replaces, which drift the day the location
//! moves.

use std::path::PathBuf;

/// The user's home directory: `%USERPROFILE%` (Windows) then `$HOME` (Unix).
/// `None` when neither is set.
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// `~/.termherd` — the app data dir (PRD §7) the stores live under. `None` when
/// no home directory is set, in which case the caller skips its file.
#[must_use]
pub fn termherd_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".termherd"))
}
