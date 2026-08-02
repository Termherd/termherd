//! A symlink is a document by name and can be a program by nature — and the OS
//! opener follows it.
//!
//! This lives outside `src/` on purpose. Creating a symlink needs
//! `std::os::unix::fs::symlink`, a compile-time OS fork, and the workspace
//! confines those to a short allow-list of audited files
//! (`scripts/check-os-cfg-containment.sh`, which scans `*/src/**` only). Adding
//! the adapter's own source to that list to accommodate a test would licence
//! OS-conditional *production* code there unnoticed — the precise thing the
//! quarantine exists to prevent. A test binary is the honest home.

#![cfg(unix)]
// `clippy.toml` already allows `expect` in tests, but that detection covers
// `#[cfg(test)]` items — an integration test is its own crate and does not
// match. Setting up a tempdir and symlinks is all fallible calls, and the
// config's own comment says the discipline should not fight the harness.
#![allow(clippy::expect_used)]

use std::fs;
use std::path::PathBuf;

use termherd_core::PathRoots;
use termherd_core::ports::PathResolver;
use termherd_scan::FsPathResolver;

/// A repo holding a real document and a real program.
fn repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).expect("mkdir");
    fs::write(repo.join("payload.app"), "").expect("write");
    fs::write(repo.join("real.md"), "").expect("write");
    (tmp, repo)
}

fn roots(cwd: &std::path::Path) -> PathRoots {
    PathRoots {
        cwd: Some(cwd.to_path_buf()),
        launch_cwd: None,
    }
}

#[test]
fn a_document_name_pointing_at_a_program_is_judged_as_the_program() {
    // The bypass this closes: `exists()` follows the link, a lexical collapse
    // does not, so a name-based policy sees `.md` while the opener reaches
    // `.app`. Git carries symlinks, so an untrusted clone is one commit away.
    let (_tmp, repo) = repo();
    std::os::unix::fs::symlink("payload.app", repo.join("notes.md")).expect("symlink");

    let found = FsPathResolver::with_home(None)
        .resolve("notes.md", &roots(&repo))
        .expect("the link points at a real file, so it resolves");

    assert_eq!(
        found.path,
        repo.join("notes.md"),
        "what opens is what was pointed at"
    );
    assert_eq!(
        found.real,
        fs::canonicalize(repo.join("payload.app")).expect("canonicalize"),
        "what policy judges is what the opener would reach"
    );
    assert!(
        termherd_core::paths::runs_on_open(&found.real),
        "so it is refused"
    );
    assert!(
        !termherd_core::paths::runs_on_open(&found.path),
        "judging the name instead would have waved it through — this is the \
         assertion that pins why the resolver reports two paths and not one"
    );
}

#[test]
fn a_symlink_to_an_ordinary_document_stays_openable() {
    // The refusal follows the target, so an innocent link is still a link —
    // otherwise every symlinked doc in a worktree would go dark.
    let (_tmp, repo) = repo();
    std::os::unix::fs::symlink("real.md", repo.join("alias.md")).expect("symlink");

    let found = FsPathResolver::with_home(None)
        .resolve("alias.md", &roots(&repo))
        .expect("resolves");
    assert!(!termherd_core::paths::runs_on_open(&found.real));
}

#[test]
fn a_dangling_symlink_is_not_a_link() {
    // `exists()` is false for a broken link, so it never reaches policy at all.
    let (_tmp, repo) = repo();
    std::os::unix::fs::symlink("gone.md", repo.join("broken.md")).expect("symlink");

    assert_eq!(
        FsPathResolver::with_home(None).resolve("broken.md", &roots(&repo)),
        None
    );
}
