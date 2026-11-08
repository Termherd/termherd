//! The sidebar key — [`sidebar_key`], the single rule that decides which
//! sidebar row a directory belongs to, applied by the walk to a session's `cwd`
//! and by a hand-added repo alike. Two rules here meant two rows for one
//! repository, which is what this module exists to prevent.
//!
//! Also the repository-root helper for the "new session in the same repo"
//! shortcut. Note that [`repo_root`] is *not* part of the key: it answers a
//! different question — where to launch — and the two were briefly conflated.

use std::path::{Path, PathBuf};

use termherd_claude::derive::collapse_worktree;

/// The repository root for `start`: the nearest ancestor (including `start`
/// itself) that holds a `.git` entry, or `None` if none does. The entry may be
/// a directory (a normal clone) or a file (a submodule or linked worktree
/// `.git` pointer), so both count.
///
/// Used by the "new Claude session in the same repo" shortcut: a session
/// may be running in a subdirectory, so the launch walks up to the repo root
/// rather than reusing the literal cwd.
#[must_use]
pub fn repo_root(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(current) = dir {
        if current.join(".git").exists() {
            return Some(current.to_owned());
        }
        dir = current.parent();
    }
    None
}

/// **The** sidebar key for a directory: the one rule, used by the scan for a
/// session's `cwd` and by a hand-added repo alike (`F-repo-add`). A worktree
/// checkout collapses onto its main project when that parent exists on disk,
/// like upstream's `fs.existsSync`; everything else is the path as given.
///
/// Deliberately *nothing* else. Two transformations were tried here and
/// removed, because the scan cannot apply them and a key it cannot produce is
/// a second sidebar row for one repository:
///
/// - **no `canonicalize`** — the walk keys on the `cwd` the CLI wrote, which is
///   the path the user's shell was standing in. Resolving symlinks would
///   diverge whenever one is in play, and *always* on Windows, where
///   canonicalisation returns the `\\?\C:\…` form no transcript ever contains.
/// - **no [`repo_root`]** — a session started in a subdirectory is keyed at
///   that subdirectory. Climbing to the repository would file a declaration
///   under a path the scan never produces for it.
#[must_use]
pub fn sidebar_key(dir: &str) -> String {
    let dir = lexical(dir);
    match collapse_worktree(&dir) {
        Some(parent) if Path::new(parent).exists() => parent.to_owned(),
        _ => dir,
    }
}

/// The one spelling of an absolute path: re-joined from its components, which
/// drops a trailing separator and any `.`, and writes the platform's own
/// separator (so `C:/dev/app` and `C:\dev\app` are one key on Windows).
///
/// This is *not* the resolution `sidebar_key` refuses. Nothing here touches the
/// disk and no symlink is followed: two spellings of one path become one
/// string, which is exactly what keeps a repository on one row. A CLI records
/// the clean spelling, so this only ever moves a hand-typed path *towards* what
/// the walk emits — and a hand-typed `~/proj/` was otherwise a second,
/// permanent row that `forget_repo` could not remove under the other spelling.
///
/// `..` is deliberately left alone: removing it lexically is wrong the moment a
/// symlink is in the path, which is the same trap `canonicalize` was removed
/// for. A relative path is returned untouched — it is rejected upstream.
fn lexical(dir: &str) -> String {
    let path = Path::new(dir);
    if !path.is_absolute() {
        return dir.to_owned();
    }
    path.components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .into_owned()
}

/// The sidebar key for a path the user picked or dropped (`F-repo-add`).
///
/// A file resolves to the directory holding it, then [`sidebar_key`] applies
/// the scan's rule and only that. `None` when the path does not exist — a row
/// whose launch buttons cannot work is worse than a refusal — or when it is
/// relative, since every key the walk produces is absolute and a relative one
/// would be read against whatever directory the app happens to be in.
#[must_use]
pub fn normalize_repo_path(picked: &Path) -> Option<PathBuf> {
    let holder = if std::fs::metadata(picked).ok()?.is_file() {
        picked.parent()?
    } else {
        picked
    };
    holder
        .is_absolute()
        .then(|| PathBuf::from(sidebar_key(&holder.to_string_lossy())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn repo_root_finds_the_nearest_dot_git_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let nested = repo.join("crates").join("app");
        fs::create_dir_all(&nested).unwrap();
        // A normal clone: `.git` is a directory at the repo root.
        fs::create_dir(repo.join(".git")).unwrap();

        // From a deep subdirectory, the walk climbs to the repo root.
        assert_eq!(repo_root(&nested).as_deref(), Some(repo.as_path()));
        // From the root itself, it returns the root.
        assert_eq!(repo_root(&repo).as_deref(), Some(repo.as_path()));
    }

    #[test]
    fn repo_root_accepts_a_dot_git_file_and_returns_none_outside_a_repo() {
        let tmp = tempfile::tempdir().unwrap();
        // A linked worktree / submodule: `.git` is a file pointer, not a dir.
        let worktree = tmp.path().join("wt");
        fs::create_dir(&worktree).unwrap();
        fs::write(worktree.join(".git"), "gitdir: /somewhere\n").unwrap();
        assert_eq!(repo_root(&worktree).as_deref(), Some(worktree.as_path()));

        // A directory with no `.git` anywhere above it has no repo root.
        let bare = tmp.path().join("bare");
        fs::create_dir(&bare).unwrap();
        assert_eq!(repo_root(&bare), None);
    }

    /// A clone with a nested subdirectory and a linked worktree beside it.
    /// Returns (repo, worktree).
    fn clone_with_a_worktree(tmp: &Path) -> (PathBuf, PathBuf) {
        let repo = tmp.join("proj");
        fs::create_dir_all(repo.join("crates").join("core")).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();

        let worktree = repo.join(".worktrees").join("feat");
        fs::create_dir_all(worktree.join("crates")).unwrap();
        fs::write(worktree.join(".git"), "gitdir: /elsewhere\n").unwrap();
        (repo, worktree)
    }

    fn as_key(path: &Path) -> String {
        path.display().to_string().replace('\\', "/")
    }

    /// The key the *walk* derives for a session whose transcript records `cwd`
    /// — the only trustworthy expectation, since it is the thing a declaration
    /// has to agree with. Writes a throwaway `~/.claude/projects` under `tmp`
    /// and runs the real scan over it.
    fn walked_key(tmp: &Path, folder: &str, cwd: &str) -> String {
        let projects = tmp.join("projects").join(folder);
        fs::create_dir_all(&projects).unwrap();
        // The transcript is named after its folder, so the session id is unique
        // across the fixtures one test writes, and the lookup below can name
        // the one it wants. Called `abc` alike, every fixture answered the
        // lookup — the scan keeps both records, it does not merge them, so the
        // predicate was true of each and `find` returned whichever the
        // filesystem enumerated first. A test asserting about one directory was
        // answered about another: the right answer on APFS, a neighbouring
        // fixture's on ext4 and on Windows, which is why it read green here and
        // failed in CI on both.
        fs::write(
            projects.join(format!("{folder}.jsonl")),
            format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":\"hi\"}}\n"),
        )
        .unwrap();
        let records = <crate::FsScanner as termherd_core::ports::ProjectScanner>::scan(
            &crate::FsScanner::new(tmp.join("projects")),
        )
        .unwrap();
        let hit = records
            .iter()
            .find(|r| r.session_id == folder)
            .expect("the scan found the session just written");
        // In the `/` spelling `as_key` uses, so a comparison is about the key
        // and not about which separator the platform writes.
        as_key(Path::new(&hit.project_path))
    }

    #[test]
    fn the_declared_key_is_the_walked_key_for_a_subdirectory() {
        // The case the first version of this test skipped, and the one the
        // removed `repo_root` step got wrong: a session started in a
        // subdirectory is keyed *at* that subdirectory, so a declaration of it
        // must be too, or the repository occupies two rows.
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = clone_with_a_worktree(tmp.path());
        let deep = repo.join("crates").join("core");

        let walked = walked_key(tmp.path(), "C--proj-crates-core", &as_key(&deep));
        // Spelled out beside the cross-check: comparing the two sides alone
        // cannot tell a wrong answer from a wrong fixture.
        assert_eq!(walked, as_key(&deep), "the walk keys at the subdirectory");
        assert_eq!(normalize_repo_path(&deep).map(|p| as_key(&p)), Some(walked));
        assert_ne!(
            normalize_repo_path(&deep),
            Some(repo.clone()),
            "climbing to the repo root is what produced the duplicate row"
        );
    }

    #[test]
    fn two_spellings_of_one_directory_are_one_key() {
        // A picker cannot produce these, an MCP caller and a hand-typed path
        // can: a trailing separator, a `.` component, or forward slashes on
        // Windows. Each was a key the walk can never emit — so a second,
        // permanent row that `forget_repo` could not reach under the other
        // spelling. Built as strings deliberately: `PathBuf::join` normalises
        // them away, which is why the sibling tests could not see this.
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = clone_with_a_worktree(tmp.path());
        let walked = walked_key(tmp.path(), "C--proj", &as_key(&repo));
        assert_eq!(walked, as_key(&repo), "the walk keys the repo at itself");

        let plain = as_key(&repo);
        for spelling in [
            format!("{plain}/"),
            format!("{plain}/."),
            format!("{plain}/./"),
        ] {
            let key = normalize_repo_path(Path::new(&spelling))
                .map(|p| as_key(&p))
                .unwrap_or_else(|| panic!("{spelling} exists"));
            assert_eq!(key, walked, "spelling {spelling:?} must not earn a row");
        }
    }

    // The symlink case lives in `tests/symlinked_repo_key.rs`: it needs an
    // OS-conditional API, and the containment gate scans `src/**` only —
    // allow-listing this file for a test would licence OS-conditional
    // *production* code here unnoticed.

    #[test]
    fn the_declared_key_is_the_walked_key_for_a_worktree_and_its_subdirectories() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, worktree) = clone_with_a_worktree(tmp.path());

        // `repo_root` stops at the worktree — its `.git` is a file — which is
        // why it is not the key rule.
        assert_eq!(repo_root(&worktree).as_deref(), Some(worktree.as_path()));

        let walked = walked_key(tmp.path(), "C--proj-worktrees-feat", &as_key(&worktree));
        assert_eq!(walked, as_key(&repo), "the walk collapses the worktree");
        assert_eq!(
            normalize_repo_path(&worktree).map(|p| as_key(&p)),
            Some(walked.clone())
        );
        // A subdirectory of a worktree is keyed at that subdirectory (no climb),
        // and the collapse does not apply to it — it is not the final component.
        let inside = worktree.join("crates");
        let walked_inside = walked_key(
            tmp.path(),
            "C--proj-worktrees-feat-crates",
            &as_key(&inside),
        );
        // Spelled out, not just compared to the other side: an expectation that
        // is only ever "whatever the walk said" cannot tell a wrong answer from
        // a wrong *fixture*, which is how this read green while the lookup was
        // picking a neighbouring transcript.
        assert_eq!(
            walked_inside,
            as_key(&inside),
            "the collapse wants the worktree as the final component"
        );
        assert_eq!(
            normalize_repo_path(&inside).map(|p| as_key(&p)),
            Some(walked_inside)
        );
    }

    #[test]
    fn normalize_takes_a_file_to_the_directory_holding_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = clone_with_a_worktree(tmp.path());
        let dir = repo.join("crates").join("core");
        let file = dir.join("lib.rs");
        fs::write(&file, "// x\n").unwrap();

        assert_eq!(normalize_repo_path(&file), Some(dir));
    }

    #[test]
    fn normalize_keeps_a_directory_that_is_in_no_repository() {
        let tmp = tempfile::tempdir().unwrap();
        let notes = tmp.path().join("notes");
        fs::create_dir(&notes).unwrap();

        // Any directory is declarable — a shell in a notes folder is a use.
        assert_eq!(normalize_repo_path(&notes), Some(notes));
    }

    #[test]
    fn normalize_rejects_a_missing_path_and_a_relative_one() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(normalize_repo_path(&tmp.path().join("gone")), None);
        // Relative: every key the walk produces is absolute, and this one would
        // be read against whatever directory the app happens to be in.
        assert_eq!(normalize_repo_path(Path::new("crates")), None);
    }
}
