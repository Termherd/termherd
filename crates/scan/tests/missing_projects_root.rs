//! A user who has not run Claude Code yet has no `~/.claude/projects`. The
//! scanner and the watch the app wires on that root must read it as "no
//! sessions yet", then pick up the directory and its first session when Claude
//! Code creates them, without a restart.
#![allow(
    clippy::expect_used,
    reason = "a test binary; `allow-expect-in-tests` only covers #[cfg(test)] items"
)]

use std::fs;
use std::sync::mpsc;
use std::time::Duration;

use termherd_core::ports::ProjectScanner;
use termherd_scan::{FsScanner, watch_changes};

#[test]
fn a_projects_root_created_after_launch_is_scanned_and_watched() {
    let home = tempfile::tempdir().expect("tempdir");
    let root = home.path().join(".claude").join("projects");
    let scanner = FsScanner::new(root.clone());
    let (tx, changes) = mpsc::channel::<()>();
    let _watch = watch_changes(root.clone(), Duration::from_millis(100), move || {
        let _ = tx.send(());
    })
    .expect("a missing root is watched, not refused");

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

    changes
        .recv_timeout(Duration::from_secs(10))
        .expect("no change signal once the root was created");
    let records = scanner.scan().expect("scan the created root");
    let paths: Vec<&str> = records
        .iter()
        .map(|record| record.project_path.as_str())
        .collect();
    assert_eq!(paths, ["/home/user/repo"]);
}
