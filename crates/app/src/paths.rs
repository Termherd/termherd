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

/// The app data dir (PRD §7). Resolved in `main()` alone and injected.
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

    /// The `N` of libtest's `test result: ok. N passed; …` line; 0 without one.
    fn passed_count(stdout: &str) -> usize {
        stdout
            .lines()
            .filter_map(|line| line.strip_prefix("test result: "))
            .find_map(|rest| {
                rest.split_once(". ")?
                    .1
                    .split_once(" passed")?
                    .0
                    .parse()
                    .ok()
            })
            .unwrap_or(0)
    }

    #[test]
    fn passed_count_reads_the_summary_line_and_nothing_else() {
        assert_eq!(passed_count("test result: ok. 617 passed; 0 failed;"), 617);
        assert_eq!(passed_count("test result: FAILED. 3 passed; 2 failed;"), 3);
        assert_eq!(passed_count("running 0 tests\n"), 0);
    }

    /// Reruns this test binary under an empty home, which must stay empty.
    #[test]
    fn no_test_writes_the_data_dir_of_the_home_it_runs_in() {
        let home = tempfile::tempdir().expect("tempdir");
        // Session ids restart at 1 per process: a shared temp dir would collide.
        let tmp = tempfile::tempdir().expect("tempdir");
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--skip",
                "no_test_writes_the_data_dir_of_the_home_it_runs_in",
            ])
            .env("USERPROFILE", home.path())
            .env("HOME", home.path())
            .env("TMPDIR", tmp.path())
            .env("TMP", tmp.path())
            .env("TEMP", tmp.path())
            .output()
            .expect("rerun the test binary");
        let stdout = String::from_utf8_lossy(&output.stdout);

        // `.termherd` is ours; `.claude` is Claude Code's, which the docs
        // editor writes into.
        let written: Vec<String> = std::fs::read_dir(home.path())
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            !written
                .iter()
                .any(|name| name == ".termherd" || name == ".claude"),
            "tests wrote the user's home: {written:?}"
        );
        assert!(
            output.status.success() && passed_count(&stdout) > 0,
            "no data dir was written, but the rerun failed, so that proves nothing; \
             the cause is the failing test reported below, not a write:\n{stdout}"
        );
    }
}
