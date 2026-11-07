//! The walk under `~/.claude/projects` never follows a symlink: a link there
//! points wherever its author chose, and what the walk reads reaches the
//! sidebar and the full-text index.
//!
//! It lives here rather than beside the walk's tests in `src/walk.rs` because
//! creating a symlink needs `std::os::unix::fs::symlink`, and
//! `check-os-cfg-containment.sh` scans `*/src/**`.
#![cfg(unix)]
#![allow(
    clippy::expect_used,
    reason = "a test binary; `allow-expect-in-tests` only covers #[cfg(test)] items"
)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use termherd_core::ports::ProjectScanner;
use termherd_scan::FsScanner;

fn transcript(cwd: &str, prompt: &str) -> String {
    format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":\"{prompt}\"}}\n")
}

/// A projects root holding one real folder with one real session, and a
/// directory outside it the links below point into.
fn projects() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("projects");
    let real = root.join("C--real");
    fs::create_dir_all(&real).expect("mkdir");
    fs::write(real.join("real.jsonl"), transcript("/real", "kept")).expect("write");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).expect("mkdir");
    (tmp, root, outside)
}

fn summaries(root: &Path) -> Vec<String> {
    let mut found: Vec<String> = FsScanner::new(root.to_owned())
        .scan()
        .expect("scan")
        .into_iter()
        .map(|r| r.digest.summary)
        .collect();
    found.sort();
    found
}

#[test]
fn a_symlinked_session_file_is_not_read() {
    let (_tmp, root, outside) = projects();
    let secret = outside.join("secret.jsonl");
    fs::write(&secret, transcript("/real", "planted")).expect("write");
    symlink(&secret, root.join("C--real").join("link.jsonl")).expect("symlink");

    assert_eq!(summaries(&root), vec!["kept".to_owned()]);
}

#[test]
fn a_symlinked_project_folder_is_not_entered() {
    let (_tmp, root, outside) = projects();
    let elsewhere = outside.join("C--elsewhere");
    fs::create_dir_all(&elsewhere).expect("mkdir");
    fs::write(
        elsewhere.join("other.jsonl"),
        transcript("/elsewhere", "planted"),
    )
    .expect("write");
    symlink(&elsewhere, root.join("C--elsewhere")).expect("symlink");

    assert_eq!(summaries(&root), vec!["kept".to_owned()]);
}

#[test]
fn a_symlinked_session_subdirectory_gives_no_cwd() {
    // The folder's own transcript carries no cwd, so the only cwd on offer is
    // behind the link; refusing it leaves the folder underivable.
    let (_tmp, root, outside) = projects();
    let folder = root.join("C--nocwd");
    fs::create_dir_all(&folder).expect("mkdir");
    fs::write(
        folder.join("abc.jsonl"),
        "{\"type\":\"user\",\"message\":\"no cwd here\"}\n",
    )
    .expect("write");
    let session_dir = outside.join("some-uuid");
    fs::create_dir_all(&session_dir).expect("mkdir");
    fs::write(session_dir.join("agent.jsonl"), transcript("/planted", "x")).expect("write");
    symlink(&session_dir, folder.join("some-uuid")).expect("symlink");

    assert_eq!(summaries(&root), vec!["kept".to_owned()]);
}

#[test]
fn a_symlinked_subagent_transcript_gives_no_cwd() {
    let (_tmp, root, outside) = projects();
    let folder = root.join("C--nocwd");
    let subagents = folder.join("some-uuid").join("subagents");
    fs::create_dir_all(&subagents).expect("mkdir");
    fs::write(
        folder.join("abc.jsonl"),
        "{\"type\":\"user\",\"message\":\"no cwd here\"}\n",
    )
    .expect("write");
    let planted = outside.join("agent.jsonl");
    fs::write(&planted, transcript("/planted", "x")).expect("write");
    symlink(&planted, subagents.join("agent.jsonl")).expect("symlink");

    assert_eq!(summaries(&root), vec!["kept".to_owned()]);
}

#[test]
fn the_real_subagent_fallback_still_derives_beside_a_refused_link() {
    // Guards the refusal from over-reaching: the same layout with a real
    // subagent transcript still names the folder's project.
    let (_tmp, root, _outside) = projects();
    let folder = root.join("C--sub");
    let subagents = folder.join("some-uuid").join("subagents");
    fs::create_dir_all(&subagents).expect("mkdir");
    fs::write(
        folder.join("abc.jsonl"),
        "{\"type\":\"user\",\"message\":\"from sub\"}\n",
    )
    .expect("write");
    fs::write(subagents.join("agent.jsonl"), transcript("/sub", "x")).expect("write");

    assert_eq!(
        summaries(&root),
        vec!["from sub".to_owned(), "kept".to_owned()]
    );
}
