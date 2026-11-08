//! Creating a git worktree for a new Claude session. A process adapter owned
//! by the shell: it runs the user's `git`, which `core` never sees.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A worktree about to be added: where it goes, and the branch it checks out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub path: PathBuf,
    pub branch: String,
}

/// Why no worktree was added.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a name git can take as a branch: {0:?}")]
    InvalidName(String),
    #[error("could not run git: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("git {step} failed: {stderr}")]
    Git { step: &'static str, stderr: String },
}

/// The worktree name for a session minted under `id`: `claude-` and the id's
/// first eight hex digits.
#[must_use]
pub fn name_for(id: &str) -> String {
    let digits: String = id
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(8)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    format!("claude-{digits}")
}

/// Whether git can take `name` as both a branch and a folder, and never reads
/// it as an option.
fn is_plain_name(name: &str) -> bool {
    !name.starts_with('-')
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The checkout that owns the repository whose shared git dir is
/// `common_dir`. A worktree started from a linked worktree still lands in the
/// main one, where the others are. A shared dir not named `.git` (a bare
/// repository, a separate git dir) names no checkout, so `root` stands in.
#[must_use]
pub fn main_checkout(common_dir: &Path, root: &Path) -> PathBuf {
    // Re-joined, so git's forward slashes become the platform's own.
    let common_dir: PathBuf = common_dir.components().collect();
    match common_dir.parent() {
        Some(main) if common_dir.file_name() == Some(OsStr::new(".git")) => main.to_owned(),
        _ => root.to_owned(),
    }
}

/// Where the worktree `name` of the checkout `main` goes: the folder
/// `claude --worktree` uses, which the scan files under `main`.
#[must_use]
pub fn plan(main: &Path, name: &str) -> Plan {
    Plan {
        path: main.join(".claude").join("worktrees").join(name),
        branch: name.to_owned(),
    }
}

/// Add the worktree `name` on a new branch off the HEAD of the checkout at
/// `root`, and return its path. Blocks on `git`: run it off the GUI thread.
///
/// # Errors
///
/// A name git could not take, a `git` that cannot be run, or a git step that
/// fails (not a repository, the branch or folder already taken).
pub fn create(root: &Path, name: &str) -> Result<PathBuf, Error> {
    if !is_plain_name(name) {
        return Err(Error::InvalidName(name.to_owned()));
    }
    let common_dir = git(
        root,
        "rev-parse",
        ["rev-parse", "--path-format=absolute", "--git-common-dir"].map(OsStr::new),
    )?;
    let plan = plan(&main_checkout(Path::new(&common_dir), root), name);
    git(
        root,
        "worktree add",
        [
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-b"),
            OsStr::new(&plan.branch),
            OsStr::new("--"),
            plan.path.as_os_str(),
        ],
    )?;
    Ok(plan.path)
}

/// Run `git -C dir args`, answering its trimmed output, or what it said on
/// failure. Its stdin is closed: a git that wants to ask something fails.
fn git<'a>(
    dir: &Path,
    step: &'static str,
    args: impl IntoIterator<Item = &'a OsStr>,
) -> Result<String, Error> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()?;
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_owned();
    if output.status.success() {
        Ok(text(&output.stdout))
    } else {
        Err(Error::Git {
            step,
            stderr: text(&output.stderr),
        })
    }
}

/// A repository with one commit, so `HEAD` names something to branch off.
#[cfg(test)]
pub(crate) fn test_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Canonical, so the paths git prints compare equal on macOS (`/private`).
    let repo = tmp.path().canonicalize().expect("canonical").join("app");
    std::fs::create_dir(&repo).expect("mkdir");
    tests::git(&repo, &["init", "-q"]);
    tests::git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
    (tmp, repo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const MINTED: &str = "3f2a9c1b-77de-4c0a-9e2b-5d1f0a6b8c4e";

    #[test]
    fn a_name_is_claude_and_the_ids_first_eight_hex_digits() {
        assert_eq!(name_for(MINTED), "claude-3f2a9c1b");
    }

    #[test]
    fn a_name_skips_the_ids_hyphens_and_lowers_its_case() {
        assert_eq!(name_for("3F2A-9C1B-0000"), "claude-3f2a9c1b");
    }

    #[test]
    fn the_main_checkout_is_the_parent_of_a_dot_git_common_dir() {
        assert_eq!(
            main_checkout(
                Path::new("/dev/app/.git"),
                Path::new("/dev/app/.worktrees/x")
            ),
            PathBuf::from("/dev/app")
        );
    }

    #[test]
    fn a_common_dir_not_named_dot_git_leaves_the_checkout_itself() {
        assert_eq!(
            main_checkout(Path::new("/srv/app.git"), Path::new("/dev/app")),
            PathBuf::from("/dev/app")
        );
    }

    #[test]
    fn a_worktree_goes_under_claudes_worktree_folder_on_a_branch_of_its_name() {
        assert_eq!(
            plan(Path::new("/dev/app"), "claude-3f2a9c1b"),
            Plan {
                path: PathBuf::from("/dev/app/.claude/worktrees/claude-3f2a9c1b"),
                branch: "claude-3f2a9c1b".to_owned(),
            }
        );
    }

    proptest! {
        #[test]
        fn any_minted_id_names_a_branch_git_takes(id in "[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}") {
            let name = name_for(&id);
            prop_assert!(name.starts_with("claude-"), "{name}");
            prop_assert_eq!(name.len(), "claude-".len() + 8);
            prop_assert!(name["claude-".len()..].chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            prop_assert!(id.replace('-', "").starts_with(&name["claude-".len()..]));
        }

        #[test]
        fn a_planned_worktree_files_under_its_main_checkout(
            main in "(/[a-z]{1,8}){1,4}",
            id in "[0-9a-f]{32}",
        ) {
            // The scan's own rule, so the session joins its project's row (FR1).
            let planned = plan(Path::new(&main), &name_for(&id));
            let path = planned.path.to_string_lossy();
            prop_assert_eq!(
                termherd_claude::derive::collapse_worktree(&path),
                Some(main.as_str())
            );
        }
    }

    /// Run git in `dir` with a fixed identity, failing the test on an error.
    pub(super) fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn repo() -> (tempfile::TempDir, PathBuf) {
        test_repo()
    }

    #[test]
    fn a_worktree_is_added_under_the_checkout_on_its_own_branch() {
        let (_tmp, repo) = repo();
        let path = create(&repo, "claude-3f2a9c1b").expect("created");
        assert_eq!(path, repo.join(".claude/worktrees/claude-3f2a9c1b"));
        assert_eq!(git(&path, &["branch", "--show-current"]), "claude-3f2a9c1b");
        assert_eq!(
            git(&path, &["rev-parse", "HEAD"]),
            git(&repo, &["rev-parse", "HEAD"]),
            "branched off the checkout's HEAD"
        );
    }

    #[test]
    fn a_worktree_asked_from_a_linked_worktree_lands_under_the_main_checkout() {
        let (_tmp, repo) = repo();
        let first = create(&repo, "claude-00000001").expect("first");
        git(
            &first,
            &["commit", "-q", "--allow-empty", "-m", "on the branch"],
        );
        let second = create(&first, "claude-00000002").expect("second");
        assert_eq!(second, repo.join(".claude/worktrees/claude-00000002"));
        assert_eq!(
            git(&second, &["rev-parse", "HEAD"]),
            git(&first, &["rev-parse", "HEAD"]),
            "branched off the linked worktree it was asked from"
        );
    }

    #[test]
    fn a_taken_name_is_refused_by_git() {
        let (_tmp, repo) = repo();
        create(&repo, "claude-3f2a9c1b").expect("first");
        let error = create(&repo, "claude-3f2a9c1b").expect_err("taken");
        assert!(matches!(error, Error::Git { .. }), "{error}");
    }

    #[test]
    fn outside_a_repository_git_refuses() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let error = create(tmp.path(), "claude-3f2a9c1b").expect_err("no repo");
        assert!(matches!(error, Error::Git { .. }), "{error}");
        assert!(!tmp.path().join(".claude").exists(), "nothing written");
    }

    #[test]
    fn a_name_git_would_read_as_an_option_never_reaches_it() {
        let (_tmp, repo) = repo();
        for name in ["-b", "--force", "a b", "../escape", ""] {
            let error = create(&repo, name).expect_err(name);
            assert!(matches!(error, Error::InvalidName(_)), "{name}: {error}");
        }
        assert!(!repo.join(".claude").exists(), "nothing written");
    }
}
