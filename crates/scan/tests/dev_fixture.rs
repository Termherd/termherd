//! The dev instance's generated Claude fixture, read back through the scanner
//! the app uses, so a fixture the scan cannot read fails here and not in a
//! screenshot.

#![allow(clippy::expect_used, reason = "test binary; see clippy.toml")]

#[path = "../examples/dev_fixture/fixture.rs"]
mod fixture;

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use termherd_core::ports::ProjectScanner;
use termherd_scan::FsScanner;

fn scan(root: &Path, now: SystemTime) -> Vec<termherd_core::SessionRecord> {
    fixture::write(root, now).expect("write the fixture");
    FsScanner::new(root.join("claude").join("projects"))
        .scan()
        .expect("scan the fixture")
}

fn name_of(project_path: &str) -> &str {
    Path::new(project_path)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
}

#[test]
fn every_session_is_found_under_its_repository_with_its_title() {
    let root = tempfile::tempdir().expect("tempdir");
    let records = scan(root.path(), SystemTime::now());

    let mut found: Vec<(&str, &str)> = records
        .iter()
        .map(|record| {
            (
                name_of(&record.project_path),
                record.digest.display_title(None),
            )
        })
        .collect();
    found.sort();
    // Spelled out rather than derived from `SESSIONS`: the worktree session
    // must land on its repository's row, and the untitled ones must fall back
    // to their first prompt — both are the scan's work, not the fixture's.
    let expected = [
        ("acme-api", "Add rate limiting to the login endpoint"),
        ("acme-api", "Fix the flaky orders integration test"),
        ("acme-api", "Release 1.4 checklist"),
        ("acme-api", "Retry failed billing webhooks"),
        ("acme-api", "Why does cargo build take so long on CI?"),
        ("acme-web", "Accessibility audit"),
        ("acme-web", "Dark mode toggle on the settings page"),
        ("acme-web", "Migrate forms to the new validation library"),
        (
            "dotfiles",
            "Set up a zsh prompt that shows the git branch and dirty state.",
        ),
    ];
    assert_eq!(found, expected);
}

#[test]
fn ages_are_relative_to_the_generation_time() {
    let day = Duration::from_secs(24 * 60 * 60);
    for now in [
        UNIX_EPOCH + Duration::from_secs(1_800_000_000),
        UNIX_EPOCH + Duration::from_secs(1_800_000_000) + 30 * day,
    ] {
        let root = tempfile::tempdir().expect("tempdir");
        let transcripts = fixture::write(root.path(), now).expect("write the fixture");
        for (session, transcript) in fixture::SESSIONS.iter().zip(&transcripts) {
            let modified = std::fs::metadata(transcript)
                .and_then(|meta| meta.modified())
                .expect("mtime");
            assert_eq!(
                now.duration_since(modified).expect("in the past"),
                Duration::from_secs(session.age_minutes * 60),
                "{}",
                session.id
            );
        }
    }
}

#[test]
fn the_colours_and_model_reach_the_digest() {
    let root = tempfile::tempdir().expect("tempdir");
    let records = scan(root.path(), SystemTime::now());
    let coloured = records
        .iter()
        .filter(|record| record.digest.agent_color.is_some())
        .count();
    assert_eq!(coloured, 4);
    assert!(
        records
            .iter()
            .all(|record| record.digest.model.as_deref() == Some("claude-opus-5-5")),
    );
}

#[test]
fn session_ids_are_unique() {
    let ids: HashSet<&str> = fixture::SESSIONS.iter().map(|session| session.id).collect();
    assert_eq!(ids.len(), fixture::SESSIONS.len());
}
