//! The Claude and Antigravity data a dev instance shows: invented repositories
//! and sessions, written as Claude Code and Antigravity write them, dated
//! relative to now.
//!
//! The sessions below are the fixture's source. Edit them to show what a check
//! or a screenshot needs; nothing here comes from a real `~/.claude` or
//! `~/.gemini/antigravity-cli`.
//!
//! Ages are relative because the sidebar dates a session by its transcript's
//! mtime (Claude) or summary timestamp (Antigravity): a frozen tree would age
//! in every screenshot, and a git checkout resets every mtime to the
//! checkout's.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use termherd_claude::path::encode_project_path;

/// The Claude Code version and model the transcripts claim, after a live
/// capture's `version` and `message.model`.
const VERSION: &str = "2.1.322";
const MODEL: &str = "claude-opus-5-5";

/// How a session got its sidebar title.
pub enum Title {
    /// Claude's own `ai-title` entry.
    Ai(&'static str),
    /// A `/rename` (`custom-title` entry).
    Custom(&'static str),
    /// None: the sidebar falls back to the first prompt.
    FirstPrompt,
}

pub struct Session {
    /// The transcript's file stem; unique across the fixture.
    pub id: &'static str,
    /// The repository, under `<root>/repos/`.
    pub repo: &'static str,
    /// A worktree of `repo`, under its `.claude/worktrees/`.
    pub worktree: Option<&'static str>,
    pub branch: &'static str,
    pub title: Title,
    /// `/color`, by Claude Code's name for it.
    pub color: Option<&'static str>,
    /// How long ago the session last wrote, in minutes.
    pub age_minutes: u64,
    /// Alternating user and assistant messages, the user's first.
    pub turns: &'static [&'static str],
}

pub const SESSIONS: &[Session] = &[
    Session {
        id: "0b7c2a4e-1f3d-4c5b-9a8e-6d2f1e0c3b71",
        repo: "acme-api",
        worktree: None,
        branch: "feat/rate-limit",
        title: Title::Ai("Add rate limiting to the login endpoint"),
        color: Some("blue"),
        age_minutes: 12,
        turns: &[
            "Add rate limiting to POST /login: 5 attempts per minute per IP.",
            "I'll add a token-bucket limiter as middleware on the auth router.",
            "Return a Retry-After header when it trips.",
            "Done: the limiter now answers 429 with Retry-After in seconds.",
        ],
    },
    Session {
        id: "2d9e4f6a-3b5c-4e7d-8f1a-0c2b4d6e8f93",
        repo: "acme-api",
        worktree: Some("billing-webhooks"),
        branch: "feat/billing-webhooks",
        title: Title::Ai("Retry failed billing webhooks"),
        color: Some("green"),
        age_minutes: 3 * 60,
        turns: &[
            "Webhook deliveries that fail should be retried with backoff.",
            "I'll queue failed deliveries and retry at 1, 5 and 30 minutes.",
        ],
    },
    Session {
        id: "4f1a6b8c-5d7e-4091-a2b3-c4d5e6f70815",
        repo: "acme-api",
        worktree: None,
        branch: "main",
        title: Title::Custom("Release 1.4 checklist"),
        color: None,
        age_minutes: 26 * 60,
        turns: &[
            "Walk me through what is left before tagging 1.4.",
            "Three items: the changelog, the migration note, and the SDK bump.",
        ],
    },
    Session {
        id: "6a3c8d0e-7f91-4b2c-8d4e-5f6a7b8c9d07",
        repo: "acme-api",
        worktree: None,
        branch: "fix/orders-test",
        title: Title::Ai("Fix the flaky orders integration test"),
        color: None,
        age_minutes: 2 * 24 * 60,
        turns: &[
            "orders_tests::checkout fails about once in ten runs on CI.",
            "It races the outbox worker; I'll wait on the event, not a sleep.",
        ],
    },
    Session {
        id: "8c5e0f2a-9b13-4d4e-af60-718293a4b5c9",
        repo: "acme-api",
        worktree: None,
        branch: "main",
        title: Title::FirstPrompt,
        color: None,
        age_minutes: 9 * 24 * 60,
        turns: &[
            "Why does cargo build take so long on CI?",
            "The cache key changes every run: it hashes the whole lockfile path.",
        ],
    },
    Session {
        id: "a1d3f5b7-c9e2-4a4b-8c6d-e8f0a2b4c6d8",
        repo: "acme-web",
        worktree: None,
        branch: "feat/dark-mode",
        title: Title::Ai("Dark mode toggle on the settings page"),
        color: Some("purple"),
        age_minutes: 40,
        turns: &[
            "Add a dark mode toggle to the settings page, saved per user.",
            "I'll read the preference into a CSS class on <html> at load.",
            "Follow the system setting until the user picks one.",
            "Done: 'system' is the default and the toggle has three states.",
        ],
    },
    Session {
        id: "c3e5a7b9-d1f4-4c6d-ae8f-a0b2c4d6e8fa",
        repo: "acme-web",
        worktree: None,
        branch: "chore/forms",
        title: Title::Ai("Migrate forms to the new validation library"),
        color: None,
        age_minutes: 3 * 24 * 60,
        turns: &[
            "Move the signup and profile forms to the new validation library.",
            "Both forms now share one schema module; the old helpers are gone.",
        ],
    },
    Session {
        id: "e5a7c9d1-f3b6-4e8f-b0a1-c2d4e6f8a0bc",
        repo: "acme-web",
        worktree: None,
        branch: "main",
        title: Title::Custom("Accessibility audit"),
        color: Some("orange"),
        age_minutes: 21 * 24 * 60,
        turns: &[
            "Audit the checkout flow for keyboard and screen-reader use.",
            "Four findings: two missing labels, a focus trap, a contrast issue.",
        ],
    },
    Session {
        id: "f7c9e1a3-b5d8-4f0a-92c3-d4e6f8a0b2de",
        repo: "dotfiles",
        worktree: None,
        branch: "main",
        title: Title::FirstPrompt,
        color: None,
        age_minutes: 6 * 24 * 60,
        turns: &[
            "Set up a zsh prompt that shows the git branch and dirty state.",
            "I'll use vcs_info with a precmd hook; no plugin manager needed.",
        ],
    },
];

pub struct AntigravitySession {
    /// The conversation UUID; unique across the fixture.
    pub id: &'static str,
    /// The repository, under `<root>/repos/`.
    pub repo: &'static str,
    /// Explicit SQLite title, or `None` to fall back to the first prompt.
    pub title: Option<&'static str>,
    /// How long ago the session last wrote, in minutes.
    pub age_minutes: u64,
    /// Alternating user and planner messages, the user's first.
    pub turns: &'static [&'static str],
}

pub const ANTIGRAVITY_SESSIONS: &[AntigravitySession] = &[
    AntigravitySession {
        id: "1b4d7e9a-2c5f-4a8b-9d0e-3f6a8b1c4d72",
        repo: "acme-api",
        title: Some("Profile slow checkout SQL queries"),
        age_minutes: 25,
        turns: &[
            "The checkout query takes 400ms under load; find the missing index.",
            "EXPLAIN shows a sequential scan on order_items(order_id); adding a composite index cuts it to 4ms.",
        ],
    },
    AntigravitySession {
        id: "3e6a9c1b-4d70-4b2e-8f5a-1c3d5e7f9024",
        repo: "acme-web",
        title: Some("Bundle size regression investigation"),
        age_minutes: 5 * 60,
        turns: &[
            "Our main JS bundle grew by 180 KB after the chart upgrade.",
            "The chart package pulled in all locales; importing the core entry drops 172 KB.",
        ],
    },
    AntigravitySession {
        id: "5a8c1e3d-6f92-4d4a-9b7c-2e4f6a8b0c46",
        repo: "dotfiles",
        title: None,
        age_minutes: 18 * 60,
        turns: &[
            "Compare ripgrep and fd ignore rules across repos.",
            "Both honour .gitignore and .ignore; fd also skips hidden directories unless -H is passed.",
        ],
    },
];

/// Write the repositories under `<root>/repos`, Claude's data under
/// `<root>/claude` (for `CLAUDE_CONFIG_DIR`), and Antigravity's data under
/// `<root>/antigravity` (for `TERMHERD_ANTIGRAVITY_DIR`), every session dated
/// from `now`. Returns each Claude session's transcript, in [`SESSIONS`] order.
pub fn write(root: &Path, now: SystemTime) -> io::Result<Vec<PathBuf>> {
    let claude_dir = root.join("claude");
    let mut transcripts = Vec::with_capacity(SESSIONS.len());
    for session in SESSIONS {
        let cwd = ensure_repo(root, session.repo, session.worktree)?
            .to_string_lossy()
            .into_owned();

        let folder = claude_dir.join("projects").join(encode_project_path(&cwd));
        std::fs::create_dir_all(&folder)?;
        let transcript = folder.join(format!("{}.jsonl", session.id));
        let last = now
            .checked_sub(Duration::from_secs(session.age_minutes * 60))
            .unwrap_or(UNIX_EPOCH);
        let body: String = lines(session, &cwd, last)
            .iter()
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(&transcript, body)?;
        std::fs::File::options()
            .write(true)
            .open(&transcript)?
            .set_modified(last)?;
        transcripts.push(transcript);
    }
    write_antigravity(root, now)?;
    Ok(transcripts)
}

fn write_antigravity(root: &Path, now: SystemTime) -> io::Result<()> {
    let agy_dir = root.join("antigravity");
    std::fs::create_dir_all(&agy_dir)?;
    let db_path = agy_dir.join("conversation_summaries.db");
    let conn = Connection::open(&db_path).map_err(io::Error::other)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS conversation_summaries (
            conversation_id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            preview TEXT NOT NULL,
            step_count INTEGER NOT NULL,
            last_modified_time TEXT NOT NULL,
            workspace_uris TEXT NOT NULL,
            nesting_depth INTEGER NOT NULL DEFAULT 0
        )",
        [],
    )
    .map_err(io::Error::other)?;

    for session in ANTIGRAVITY_SESSIONS {
        let cwd = ensure_repo(root, session.repo, None)?;
        let last = now
            .checked_sub(Duration::from_secs(session.age_minutes * 60))
            .unwrap_or(UNIX_EPOCH);
        let workspace_uris = json!([path_to_file_uri(&cwd)]).to_string();
        let step_count = i64::try_from(session.turns.len()).unwrap_or(0);
        conn.execute(
            "INSERT OR REPLACE INTO conversation_summaries
             (conversation_id, title, preview, step_count, last_modified_time, workspace_uris, nesting_depth)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![
                session.id,
                session.title.unwrap_or(""),
                "",
                step_count,
                rfc3339(last),
                workspace_uris,
            ],
        )
        .map_err(io::Error::other)?;

        let logs_dir = agy_dir
            .join("brain")
            .join(session.id)
            .join(".system_generated")
            .join("logs");
        std::fs::create_dir_all(&logs_dir)?;
        let transcript = logs_dir.join("transcript.jsonl");
        let body: String = antigravity_lines(session, last)
            .iter()
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(&transcript, body)?;
        std::fs::File::options()
            .write(true)
            .open(&transcript)?
            .set_modified(last)?;
    }
    Ok(())
}

fn antigravity_lines(session: &AntigravitySession, last: SystemTime) -> Vec<Value> {
    let count = session.turns.len() as u64;
    session
        .turns
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let at = last
                .checked_sub(Duration::from_secs((count - 1 - index as u64) * 60))
                .unwrap_or(UNIX_EPOCH);
            if index % 2 == 0 {
                json!({
                    "step_index": index,
                    "source": "USER_EXPLICIT",
                    "type": "USER_INPUT",
                    "status": "DONE",
                    "created_at": rfc3339(at),
                    "content": format!("<USER_REQUEST>\n{text}\n</USER_REQUEST>"),
                })
            } else {
                json!({
                    "step_index": index,
                    "source": "MODEL",
                    "type": "PLANNER_RESPONSE",
                    "status": "DONE",
                    "created_at": rfc3339(at),
                    "content": text,
                })
            }
        })
        .collect()
}

/// Convert a local filesystem path into a `file://` URI using forward slashes
/// and percent-encoding URI delimiters, matching Antigravity's `workspace_uris`.
fn path_to_file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::with_capacity(raw.len() + 16);
    uri.push_str("file://");
    if !raw.starts_with('/') {
        uri.push('/');
    }
    for ch in raw.chars() {
        match ch {
            ' ' => uri.push_str("%20"),
            '%' => uri.push_str("%25"),
            '#' => uri.push_str("%23"),
            '?' => uri.push_str("%3F"),
            _ => uri.push(ch),
        }
    }
    uri
}

/// Create `<root>/repos/<repo>` (with a `.git` marker so repo resolution
/// recognises it) and, when `worktree` is set, its `.claude/worktrees/<name>`
/// subdirectory, returning the session's working directory.
fn ensure_repo(root: &Path, repo: &str, worktree: Option<&str>) -> io::Result<PathBuf> {
    let repo_dir = root.join("repos").join(repo);
    std::fs::create_dir_all(repo_dir.join(".git"))?;
    let cwd = match worktree {
        Some(name) => repo_dir.join(".claude").join("worktrees").join(name),
        None => repo_dir,
    };
    std::fs::create_dir_all(&cwd)?;
    Ok(cwd)
}

/// The transcript's entries, the last message stamped `last` and each one
/// before it a minute earlier.
fn lines(session: &Session, cwd: &str, last: SystemTime) -> Vec<Value> {
    let count = session.turns.len() as u64;
    let mut entries = Vec::new();
    let mut parent: Option<String> = None;
    for (index, text) in session.turns.iter().enumerate() {
        let at = last
            .checked_sub(Duration::from_secs((count - 1 - index as u64) * 60))
            .unwrap_or(UNIX_EPOCH);
        let uuid = format!("{}-{index:04}", &session.id[..23]);
        let mut entry = json!({
            "parentUuid": parent,
            "isSidechain": false,
            "userType": "external",
            "entrypoint": "cli",
            "cwd": cwd,
            "sessionId": session.id,
            "version": VERSION,
            "gitBranch": session.branch,
            "uuid": uuid,
            "timestamp": rfc3339(at),
        });
        if index % 2 == 0 {
            entry["type"] = json!("user");
            entry["message"] = json!({
                "role": "user",
                "content": [{ "type": "text", "text": text }],
            });
        } else {
            entry["type"] = json!("assistant");
            entry["effort"] = json!("medium");
            entry["message"] = json!({
                "id": format!("msg_{}", uuid.replace('-', "")),
                "type": "message",
                "role": "assistant",
                "model": MODEL,
                "content": [{ "type": "text", "text": text }],
            });
        }
        entries.push(entry);
        parent = Some(uuid);
    }
    match session.title {
        Title::Ai(title) => entries.push(json!({
            "type": "ai-title", "aiTitle": title, "sessionId": session.id,
        })),
        Title::Custom(title) => entries.push(json!({
            "type": "custom-title", "customTitle": title, "sessionId": session.id,
        })),
        Title::FirstPrompt => {}
    }
    if let Some(color) = session.color {
        entries.push(json!({
            "type": "agent-color", "agentColor": color, "sessionId": session.id,
        }));
    }
    entries
}

/// `2026-11-11T10:04:05.000Z`, the form Claude Code stamps entries with.
fn rfc3339(at: SystemTime) -> String {
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// `civil_from_days`, for the non-negative days a `SystemTime` past the epoch
/// gives).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}
