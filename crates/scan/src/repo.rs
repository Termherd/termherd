//! Repository-root helper for the "new session in the same repo" shortcut.
//! An independent leaf — no walk, cache, or codec dependency.

use std::path::{Path, PathBuf};

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
}
