//! The sidebar key must survive a symlink (`F-repo-add`).
//!
//! The `cwd` a CLI records is the path the user's shell was standing in, which
//! may run through a symlink. Resolving it on the declaration side and not on
//! the walk's side files one repository under two keys — two sidebar rows, no
//! error. On Windows the same divergence fires on *every* repository, because
//! canonicalisation there returns the `\\?\C:\…` form no transcript contains.
//!
//! It lives here rather than beside its siblings in `src/repo.rs` because it
//! needs an OS-conditional API, and `check-os-cfg-containment.sh` scans
//! `*/src/**`: allow-listing the adapter's own source for a test would licence
//! OS-conditional production code there unnoticed.

// `clippy.toml`'s `allow-expect-in-tests` recognises `#[cfg(test)]` items, not
// a `tests/` binary, so this file asks for the same latitude explicitly.
#![allow(clippy::unwrap_used, reason = "test binary; see clippy.toml")]

use std::fs;
use std::path::Path;

use termherd_core::ports::ProjectScanner;
use termherd_scan::{FsScanner, normalize_repo_path};

fn as_key(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// The key the *walk* derives for a session whose transcript records `cwd` —
/// the only trustworthy expectation, since it is what a declaration has to
/// agree with. A hand-written one would keep passing if both rules drifted.
fn walked_key(root: &Path, folder: &str, cwd: &str) -> String {
    let projects = root.join("projects");
    let dir = projects.join(folder);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("abc.jsonl"),
        format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":\"hi\"}}\n"),
    )
    .unwrap();
    let records = FsScanner::new(projects).scan().unwrap();
    // In the `/` spelling `as_key` uses: the key takes the platform's own
    // separator, and this comparison is about the key, not the separator.
    as_key(Path::new(&records[0].project_path))
}

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) {
    // Needs Developer Mode or elevation; skipped rather than failed below.
    let _ = std::os::windows::fs::symlink_dir(target, link);
}

#[test]
fn the_declared_key_is_the_walked_key_through_a_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("proj");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();

    let link = tmp.path().join("link-to-proj");
    link_dir(&repo, &link);
    if !link.exists() {
        // Windows without symlink privilege: nothing to assert, and failing
        // here would report a missing OS permission as a defect in the rule.
        eprintln!("symlink creation unavailable; skipping");
        return;
    }

    let walked = walked_key(tmp.path(), "C--link-to-proj", &as_key(&link));
    assert_eq!(
        walked,
        as_key(&link),
        "the walk keys on the cwd as written, symlink and all"
    );
    assert_eq!(
        normalize_repo_path(&link).map(|p| as_key(&p)),
        Some(walked),
        "so the declaration must not resolve it either"
    );
}
