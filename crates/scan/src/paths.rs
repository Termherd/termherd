//! Turning a path-shaped run of terminal text into a file that exists.
//!
//! `core::paths::detect` is deliberately syntactic: it cannot tell `and/or`
//! from `src/main.rs`, because nothing in the text can. **The filesystem check
//! here is what makes the feature usable** — without it, prose lights up half
//! the screen the moment the modifier is held.
//!
//! The other half of the job is that "relative to what?" has no single answer.
//! `cargo` prints relative to the workspace root, `git` to the repo root,
//! `pytest` to wherever it was invoked. So a candidate is tried against several
//! roots, innermost first: the directory the session is in now, then the
//! repository containing it, then the directory it was launched in.

use std::path::{Path, PathBuf};

use termherd_core::PathRoots;
use termherd_core::ports::PathResolver;

use crate::repo::repo_root;

/// Resolves terminal path candidates against the real filesystem.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsPathResolver;

impl PathResolver for FsPathResolver {
    fn resolve(&self, _candidate: &str, _roots: &PathRoots) -> Option<PathBuf> {
        None
    }
}

/// The roots to try for a relative candidate, innermost first and without
/// repeats: the live directory, the repository holding it, then the launch
/// directory.
fn search_roots(_roots: &PathRoots) -> Vec<PathBuf> {
    Vec::new()
}

/// Expand a leading `~` against the user's home directory.
fn expand_home(_candidate: &str) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A repo at `<tmp>/repo` with `.git`, `src/main.rs` and `crates/pty/lib.rs`.
    fn repo(tmp: &Path) -> PathBuf {
        let repo = tmp.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(repo.join("crates").join("pty")).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();
        fs::write(repo.join("src").join("main.rs"), "").unwrap();
        fs::write(repo.join("crates").join("pty").join("lib.rs"), "").unwrap();
        repo
    }

    fn roots(cwd: Option<&Path>, launch: Option<&Path>) -> PathRoots {
        PathRoots {
            cwd: cwd.map(Path::to_path_buf),
            launch_cwd: launch.map(Path::to_path_buf),
        }
    }

    #[test]
    fn prose_that_looks_like_a_path_resolves_to_nothing() {
        // The test the whole feature rests on: `and/or` and `http/2` are
        // syntactically indistinguishable from a relative path, and this is
        // the only thing that stops them underlining.
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        for prose in ["and/or", "http/2", "a/b", "N/A"] {
            assert_eq!(
                FsPathResolver.resolve(prose, &roots(Some(&repo), None)),
                None,
                "{prose} must not resolve"
            );
        }
    }

    #[test]
    fn an_absolute_path_resolves_without_any_root() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        let abs = repo.join("src").join("main.rs");
        assert_eq!(
            FsPathResolver.resolve(&abs.to_string_lossy(), &PathRoots::default()),
            Some(abs)
        );
    }

    #[test]
    fn an_absolute_path_that_does_not_exist_resolves_to_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope.rs");
        assert_eq!(
            FsPathResolver.resolve(&missing.to_string_lossy(), &PathRoots::default()),
            None
        );
    }

    #[test]
    fn a_path_relative_to_the_repo_root_resolves_from_a_subdirectory() {
        // The motivating case: `cargo test` run from `crates/pty` still prints
        // `crates/pty/lib.rs`, relative to the workspace root.
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        let deep = repo.join("crates").join("pty");
        assert_eq!(
            FsPathResolver.resolve("crates/pty/lib.rs", &roots(Some(&deep), None)),
            Some(repo.join("crates").join("pty").join("lib.rs"))
        );
    }

    #[test]
    fn the_innermost_root_wins_when_two_would_resolve() {
        // `lib.rs` exists both at `crates/pty/lib.rs` and (below) at the repo
        // root. The one relative to where the session *is* is the one meant.
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        fs::write(repo.join("lib.rs"), "").unwrap();
        let deep = repo.join("crates").join("pty");
        assert_eq!(
            FsPathResolver.resolve("lib.rs", &roots(Some(&deep), None)),
            Some(deep.join("lib.rs")),
            "the cwd beats the repo root"
        );
    }

    #[test]
    fn the_launch_directory_is_the_last_resort() {
        // After `cd /tmp` the live cwd resolves nothing, and `/tmp` has no
        // repo — only the directory the session started in still knows.
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        assert_eq!(
            FsPathResolver.resolve("src/main.rs", &roots(Some(&elsewhere), Some(&repo))),
            Some(repo.join("src").join("main.rs"))
        );
    }

    #[test]
    fn a_candidate_with_no_roots_at_all_resolves_to_nothing() {
        // A session whose directory was never known: nothing to be relative to,
        // and no panic for it.
        assert_eq!(
            FsPathResolver.resolve("src/main.rs", &PathRoots::default()),
            None
        );
    }

    #[test]
    fn a_dot_prefixed_candidate_resolves_against_the_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        assert_eq!(
            FsPathResolver.resolve("./src/main.rs", &roots(Some(&repo), None)),
            Some(repo.join("src").join("main.rs"))
        );
    }

    #[test]
    fn a_tilde_candidate_expands_against_the_home_directory() {
        // `~/…` is the shell's spelling, not the filesystem's: it means nothing
        // joined onto a root, so it is expanded rather than tried.
        let Some(home) = expand_home("~") else {
            return; // no home on this host; nothing to assert
        };
        assert!(home.is_absolute());
        assert_eq!(
            expand_home("~/x").as_deref(),
            Some(home.join("x").as_path())
        );
        assert_eq!(expand_home("relative/x"), None, "only `~` expands");
    }

    #[test]
    fn the_search_roots_are_ordered_innermost_first_and_deduplicated() {
        // A session that never left its repo root would otherwise stat the
        // same directory three times per candidate.
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        assert_eq!(
            search_roots(&roots(Some(&repo), Some(&repo))),
            vec![repo.clone()],
            "cwd, its repo root and the launch dir coincide"
        );
        let deep = repo.join("crates").join("pty");
        assert_eq!(
            search_roots(&roots(Some(&deep), Some(&repo))),
            vec![deep, repo],
            "cwd, then the repo holding it, then the launch directory"
        );
    }

    #[test]
    fn a_directory_is_a_valid_target() {
        // `ls crates/pty` prints a directory; opening it is meaningful.
        let tmp = tempfile::tempdir().unwrap();
        let repo = repo(tmp.path());
        assert_eq!(
            FsPathResolver.resolve("crates/pty", &roots(Some(&repo), None)),
            Some(repo.join("crates").join("pty"))
        );
    }

    #[test]
    fn repo_root_is_only_consulted_when_the_cwd_is_in_one() {
        // Outside any repository there is no second root to fall back to, and
        // the walk must stop rather than climbing to the filesystem root.
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("bare");
        fs::create_dir(&bare).unwrap();
        assert!(repo_root(&bare).is_none());
        assert_eq!(search_roots(&roots(Some(&bare), None)), vec![bare]);
    }
}
