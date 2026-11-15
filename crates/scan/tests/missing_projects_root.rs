//! A user who has not run Claude Code yet has no `~/.claude/projects`. The
//! scanner must read it as "no sessions yet", then find the directory and its
//! first session once Claude Code creates them, without being rebuilt.
#![allow(
    clippy::expect_used,
    reason = "a test binary; `allow-expect-in-tests` only covers #[cfg(test)] items"
)]

use std::fs;

use termherd_core::ports::ProjectScanner;
use termherd_scan::FsScanner;

#[test]
fn a_projects_root_created_after_launch_is_scanned() {
    let home = tempfile::tempdir().expect("tempdir");
    let root = home.path().join(".claude").join("projects");
    let scanner = FsScanner::new(root.clone());

    assert!(
        scanner.scan().expect("a missing root scans").is_empty(),
        "sessions out of nowhere"
    );

    // What Claude Code's first run writes: the root, a project folder, and
    // the session's transcript.
    let folder = root.join("-home-user-repo");
    fs::create_dir_all(&folder).expect("create the projects root");
    fs::write(
        folder.join("0b7c2a4e-1f3d-4c5b-9a8e-6d2f1e0c3b71.jsonl"),
        "{\"type\":\"user\",\"cwd\":\"/home/user/repo\",\"message\":\"hello\"}\n",
    )
    .expect("write the transcript");

    let records = scanner.scan().expect("scan the created root");
    let paths: Vec<&str> = records
        .iter()
        .map(|record| record.project_path.as_str())
        .collect();
    assert_eq!(paths, ["/home/user/repo"]);
}
