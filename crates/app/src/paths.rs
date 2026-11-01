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

    /// A data dir rooted at `root`: a test's own.
    #[cfg(test)]
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self(root.into())
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

    /// Set in the rerun below, so it does not rerun itself.
    const RERUN: &str = "TERMHERD_HOME_GUARD_RERUN";

    /// Reruns this whole test binary under an empty home: a test that persists
    /// through the user's data dir is caught by the dir it leaves behind, so no
    /// list of writers has to be kept by hand.
    #[test]
    fn no_test_writes_the_data_dir_of_the_home_it_runs_in() {
        if std::env::var_os(RERUN).is_some() {
            return;
        }
        let home = tempfile::tempdir().expect("tempdir");
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--skip",
                "no_test_writes_the_data_dir_of_the_home_it_runs_in",
            ])
            .env("USERPROFILE", home.path())
            .env("HOME", home.path())
            .env(RERUN, "1")
            .output()
            .expect("rerun the test binary");
        let stdout = String::from_utf8_lossy(&output.stdout);
        // A rerun that ran nothing would leave the dir absent as well.
        assert!(
            stdout.contains(" passed;"),
            "the rerun must run the suite:\n{stdout}"
        );

        let data_dir = home.path().join(".termherd");
        let written: Vec<String> = std::fs::read_dir(&data_dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            !data_dir.exists(),
            "tests wrote the user's data dir: {written:?}"
        );
    }
}
