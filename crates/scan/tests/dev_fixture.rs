//! The dev instance's generated Claude and Antigravity fixtures, read back
//! through the scanner the app uses, so a fixture the scan cannot read fails
//! here and not in a screenshot.

#![allow(clippy::expect_used, reason = "test binary; see clippy.toml")]

#[path = "../examples/dev_fixture/fixture.rs"]
mod fixture;

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use termherd_core::Provider;
use termherd_core::ports::ProjectScanner;
use termherd_scan::FsScanner;

fn scan(root: &Path, now: SystemTime) -> Vec<termherd_core::SessionRecord> {
    fixture::write(root, now).expect("write the fixture");
    FsScanner::new(root.join("claude").join("projects"))
        .with_antigravity_dir(root.join("antigravity"))
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

    let mut found: Vec<(Provider, &str, &str)> = records
        .iter()
        .map(|record| {
            (
                record.provider,
                name_of(&record.project_path),
                record.digest.display_title(None),
            )
        })
        .collect();
    found.sort_by(|a, b| (a.0.id(), a.1, a.2).cmp(&(b.0.id(), b.1, b.2)));
    // Spelled out rather than derived from the fixture tables: the worktree
    // session must land on its repository's row, the untitled sessions must
    // fall back to their first prompt, and Antigravity sessions must resolve
    // their workspace URI and SQLite/transcript title — all of which are the
    // scan's work, not the fixture's.
    let expected = [
        (
            Provider::Antigravity,
            "acme-api",
            "Profile slow checkout SQL queries",
        ),
        (
            Provider::Antigravity,
            "acme-web",
            "Bundle size regression investigation",
        ),
        (
            Provider::Antigravity,
            "dotfiles",
            "Compare ripgrep and fd ignore rules across repos.",
        ),
        (
            Provider::Claude,
            "acme-api",
            "Add rate limiting to the login endpoint",
        ),
        (
            Provider::Claude,
            "acme-api",
            "Fix the flaky orders integration test",
        ),
        (Provider::Claude, "acme-api", "Release 1.4 checklist"),
        (
            Provider::Claude,
            "acme-api",
            "Retry failed billing webhooks",
        ),
        (
            Provider::Claude,
            "acme-api",
            "Why does cargo build take so long on CI?",
        ),
        (Provider::Claude, "acme-web", "Accessibility audit"),
        (
            Provider::Claude,
            "acme-web",
            "Dark mode toggle on the settings page",
        ),
        (
            Provider::Claude,
            "acme-web",
            "Migrate forms to the new validation library",
        ),
        (
            Provider::Claude,
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

        let records = FsScanner::new(root.path().join("claude").join("projects"))
            .with_antigravity_dir(root.path().join("antigravity"))
            .scan()
            .expect("scan the fixture");
        for session in fixture::ANTIGRAVITY_SESSIONS {
            let record = records
                .iter()
                .find(|r| r.provider == Provider::Antigravity && r.session_id == session.id)
                .expect("antigravity session scanned");
            let modified = record.modified.expect("antigravity modified timestamp");
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
    let claude: Vec<_> = records
        .iter()
        .filter(|record| record.provider == Provider::Claude)
        .collect();
    let coloured = claude
        .iter()
        .filter(|record| record.digest.agent_color.is_some())
        .count();
    assert_eq!(coloured, 4);
    assert!(
        claude
            .iter()
            .all(|record| record.digest.model.as_deref() == Some("claude-opus-5-5")),
    );

    let antigravity: Vec<_> = records
        .iter()
        .filter(|record| record.provider == Provider::Antigravity)
        .collect();
    assert_eq!(antigravity.len(), fixture::ANTIGRAVITY_SESSIONS.len());
    assert!(
        antigravity.iter().all(
            |record| record.digest.message_count >= 2 && !record.digest.text_content.is_empty()
        )
    );
}

#[test]
fn session_ids_are_unique() {
    let ids: HashSet<&str> = fixture::SESSIONS
        .iter()
        .map(|session| session.id)
        .chain(
            fixture::ANTIGRAVITY_SESSIONS
                .iter()
                .map(|session| session.id),
        )
        .collect();
    assert_eq!(
        ids.len(),
        fixture::SESSIONS.len() + fixture::ANTIGRAVITY_SESSIONS.len()
    );
}
