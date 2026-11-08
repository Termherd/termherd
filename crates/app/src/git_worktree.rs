//! Creating a git worktree for a new Claude session. A process adapter owned
//! by the shell: it runs the user's `git`, which `core` never sees.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Why no worktree was added.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
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
pub(crate) fn name_for(id: &str) -> String {
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
        && !name.ends_with('-')
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The checkout that owns the repository whose shared git dir is
/// `common_dir`, as `git -C root` printed it: relative to `root` when git
/// prints it so. A worktree started from a linked worktree still lands in the
/// main one, where the others are. A shared dir not named `.git` (a bare
/// repository, a separate git dir) names no checkout, so `root` stands in.
fn main_checkout(common_dir: &Path, root: &Path) -> PathBuf {
    // Re-joined, so git's forward slashes become the platform's own.
    let common_dir: PathBuf = root.join(common_dir).components().collect();
    match common_dir.parent() {
        Some(main) if common_dir.file_name() == Some(OsStr::new(".git")) => main.to_owned(),
        _ => root.to_owned(),
    }
}

/// Where the worktree `name` of the checkout `main` goes: the folder
/// `claude --worktree` uses, which the scan files under `main`.
fn worktree_path(main: &Path, name: &str) -> PathBuf {
    main.join(".claude").join("worktrees").join(name)
}

/// Add the worktree `name` on a new branch of the same name, off the HEAD of
/// the checkout at `root`, and return its path. Blocks on `git`: run it off the GUI thread.
///
/// # Errors
///
/// A name git could not take, a `git` that cannot be run, or a git step that
/// fails (not a repository, the branch or folder already taken).
pub(crate) fn create(root: &Path, name: &str) -> Result<PathBuf, Error> {
    if !is_plain_name(name) {
        return Err(Error::InvalidName(name.to_owned()));
    }
    let path = worktree_path(&main_of(root)?, name);
    git(
        root,
        "worktree add",
        [
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-b"),
            OsStr::new(name),
            OsStr::new("--"),
            path.as_os_str(),
        ],
    )?;
    Ok(path)
}

/// The main checkout of the repo `root` is in, spelled as the user reached it
/// whenever that can be known: git answers with symlinks resolved, and a
/// different spelling of one repo is a second sidebar row.
fn main_of(root: &Path) -> Result<PathBuf, Error> {
    if root.join(".git").is_dir() {
        return Ok(root.to_owned());
    }
    // A linked worktree in a folder the scan collapses: the scan's own key.
    let key = PathBuf::from(termherd_scan::sidebar_key(&root.to_string_lossy()));
    if key != root {
        return Ok(key);
    }
    let common_dir = git(
        root,
        "rev-parse",
        // Not `--path-format=absolute`: a git older than 2.31 prints the flag
        // back as output instead of refusing it.
        ["rev-parse", "--git-common-dir"].map(OsStr::new),
    )?;
    Ok(main_checkout(Path::new(&common_dir), root))
}

/// The variables that point git at a repository other than `-C`'s, which
/// termherd inherits when started from inside a git hook.
const REPO_OVERRIDES: [&str; 4] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
];

/// `git -C dir`, deaf to [`REPO_OVERRIDES`], with its stdin closed: a git that
/// wants to ask something fails rather than waits.
fn git_in(dir: &Path) -> Command {
    let mut command = Command::new("git");
    for key in REPO_OVERRIDES {
        command.env_remove(key);
    }
    command.arg("-C").arg(dir).stdin(Stdio::null());
    command
}

/// Run `git -C dir args`, answering its trimmed output, or what it said on
/// failure.
fn git<'a>(
    dir: &Path,
    step: &'static str,
    args: impl IntoIterator<Item = &'a OsStr>,
) -> Result<String, Error> {
    let output = git_in(dir).args(args).output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(Error::Git {
            step,
            stderr: said(&String::from_utf8_lossy(&output.stderr)),
        })
    }
}

/// What a failed git said, on one line: its `hint:` advice is left out, since
/// a notice has room for the error alone.
fn said(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("hint:"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A repository with one commit, so `HEAD` names something to branch off.
#[cfg(test)]
pub(crate) fn test_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Not canonicalised: on macOS the temp dir is reached through the `/var`
    // symlink, so every test proves a worktree keeps the repo's spelling.
    let repo = tmp.path().join("app");
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
    fn a_relative_common_dir_is_read_from_the_checkout_git_ran_in() {
        assert_eq!(
            main_checkout(Path::new(".git"), Path::new("/dev/app")),
            PathBuf::from("/dev/app")
        );
    }

    #[test]
    fn a_failure_is_said_on_one_line_without_gits_hints() {
        assert_eq!(
            said("hint: Using 'master'\nhint: ...\nfatal: a branch named 'x' already exists\n"),
            "fatal: a branch named 'x' already exists"
        );
        assert_eq!(said("error: one\nerror: two\n"), "error: one error: two");
    }

    #[test]
    fn a_linked_worktree_outside_the_scans_folders_asks_git_for_its_repo() {
        let (tmp, repo) = test_repo();
        // Resolved as a shell reports it: on Windows without the verbatim
        // `\\?\` prefix, which git cannot create directories under.
        let elsewhere =
            PathBuf::from(crate::shell::resolved(&tmp.path().to_string_lossy())).join("elsewhere");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "side",
                &elsewhere.to_string_lossy(),
            ],
        );
        let made = create(&elsewhere, "claude-00000003").expect("made");
        // git's answer, so the spelling with symlinks resolved.
        let repo = PathBuf::from(crate::shell::resolved(&repo.to_string_lossy()));
        assert_eq!(made, repo.join(".claude/worktrees/claude-00000003"));
    }

    #[test]
    fn git_never_reads_a_repository_from_the_environment() {
        let command = git_in(Path::new("/dev/app"));
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        assert_eq!(removed.len(), REPO_OVERRIDES.len(), "{removed:?}");
        for key in REPO_OVERRIDES {
            assert!(removed.iter().any(|r| r == key), "{key} is removed");
        }
    }

    #[test]
    fn a_common_dir_not_named_dot_git_leaves_the_checkout_itself() {
        assert_eq!(
            main_checkout(Path::new("/srv/app.git"), Path::new("/dev/app")),
            PathBuf::from("/dev/app")
        );
    }

    #[test]
    fn a_worktree_goes_under_claudes_worktree_folder() {
        assert_eq!(
            worktree_path(Path::new("/dev/app"), "claude-3f2a9c1b"),
            PathBuf::from("/dev/app/.claude/worktrees/claude-3f2a9c1b")
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
            let planned = worktree_path(Path::new(&main), &name_for(&id));
            let path = planned.to_string_lossy();
            prop_assert_eq!(
                termherd_claude::derive::collapse_worktree(&path),
                Some(main.as_str())
            );
        }
    }

    /// Run git in `dir` with a fixed identity, failing the test on an error.
    pub(super) fn git(dir: &Path, args: &[&str]) -> String {
        let out = git_in(dir)
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

    #[test]
    fn a_worktree_is_added_under_the_checkout_on_its_own_branch() {
        let (_tmp, repo) = test_repo();
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
        let (_tmp, repo) = test_repo();
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
    fn a_created_worktree_is_filed_under_its_repo_by_the_scans_key() {
        let (_tmp, repo) = test_repo();
        let path = create(&repo, "claude-3f2a9c1b").expect("created");
        assert_eq!(
            termherd_scan::sidebar_key(&path.to_string_lossy()),
            repo.to_string_lossy()
        );
    }

    #[test]
    fn a_taken_name_is_refused_by_git() {
        let (_tmp, repo) = test_repo();
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
        let (_tmp, repo) = test_repo();
        for name in ["-b", "--force", "a b", "../escape", "", "claude-"] {
            let error = create(&repo, name).expect_err(name);
            assert!(matches!(error, Error::InvalidName(_)), "{name}: {error}");
        }
        assert!(!repo.join(".claude").exists(), "nothing written");
    }
}
