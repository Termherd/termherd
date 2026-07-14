//! Shared test helpers for the `app` submodule test suites.

use termherd_claude::digest::SessionDigest;

use crate::browser::SessionRecord;
use crate::capture::CaptureDump;

use super::*;

/// A browsed session record with the given id / project / first-prompt summary.
pub(crate) fn record(id: &str, path: &str, summary: &str) -> SessionRecord {
    SessionRecord {
        session_id: id.into(),
        project_path: path.into(),
        digest: SessionDigest {
            summary: summary.into(),
            message_count: 1,
            text_content: String::new(),
            slug: None,
            custom_title: None,
            ai_title: None,
            tail: Vec::new(),
        },
        modified: None,
    }
}

/// `count` sessions in `/p`, freshest first, applied with a scan.
pub(crate) fn scanned_group(app: &mut App, count: usize) {
    let records = (0..count)
        .map(|i| {
            let mut r = record(&format!("s{i}"), "/p", "routine work");
            r.modified = Some(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 - i as u64),
            );
            r
        })
        .collect();
    app.apply(Event::ScanCompleted(records));
}

/// Launch a plain shell tab and return its session id.
pub(crate) fn launch(app: &mut App, title: &str) -> SessionId {
    match app
        .apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            launch: Launch::Shell,
            title: title.into(),
        }))
        .as_slice()
    {
        [Effect::Spawn(spec)] => spec.session,
        other => panic!("expected Spawn, got {other:?}"),
    }
}

/// Launch a Claude session and return its id — companion to `launch`, which
/// spawns a plain shell.
pub(crate) fn launch_claude(app: &mut App) -> SessionId {
    match app
        .apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            launch: Launch::Claude { resume: None },
            title: "claude".into(),
        }))
        .as_slice()
    {
        [Effect::Spawn(spec)] => spec.session,
        other => panic!("expected Spawn, got {other:?}"),
    }
}

/// The single `Effect::Notify` a `SessionNotified` event should produce, or
/// `None` if the policy dropped it. Panics on any other effect shape so a
/// regression that emits the wrong effect fails loudly.
pub(crate) fn notify_effect(effects: &[Effect]) -> Option<(&str, &str)> {
    match effects {
        [] => None,
        [Effect::Notify { title, body }] => Some((title, body)),
        other => panic!("expected at most one Notify, got {other:?}"),
    }
}

/// The single `Effect::Capture` payload a `Capture` event should produce.
/// Panics on any other effect shape so a regression fails loudly.
pub(crate) fn capture_dump(effects: &[Effect]) -> &CaptureDump {
    match effects {
        [Effect::Capture(dump)] => dump,
        other => panic!("expected one Capture effect, got {other:?}"),
    }
}
