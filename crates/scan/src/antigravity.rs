//! Antigravity (`agy`) session discovery and transcript indexing.
//!
//! Reads `~/.gemini/antigravity-cli/conversation_summaries.db` strictly
//! read-only (`mode=ro&immutable=1`) for conversation ids, workspace URIs,
//! titles, and timestamps, then incrementally digests
//! `brain/<conversation-id>/.system_generated/logs/transcript.jsonl` through
//! [`ScanCache`] for summary fallback, message counts, and FTS search text.
//!
//! Any missing database, unreadable file, or schema drift degrades to an empty
//! list (logged, never fatal) so Antigravity discovery can never break Claude
//! session scanning.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};
use termherd_claude::digest::SessionDigest;
use termherd_core::{Provider, SessionRecord};
use tracing::{debug, warn};

use crate::cache::{CachedDigest, ScanCache, file_sig};
use crate::entry::read_transcript;
use crate::repo::sidebar_key;

/// Maximum UTF-16 length of the summary, matching `termherd_claude::digest`.
const SUMMARY_MAX_UNITS: usize = 120;
/// Per-step contribution to `text_content`.
const TEXT_SLICE_UNITS: usize = 500;
/// Full-text index budget per transcript (~8 KB).
const TEXT_CONTENT_MAX_UNITS: usize = 8000;
/// Number of trailing message snippets kept for the hover card preview.
const TAIL_LINES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryRow {
    conversation_id: String,
    title: String,
    preview: String,
    step_count: u32,
    last_modified_time: String,
    workspace_uris: String,
}

/// Scan an Antigravity data directory (normally `~/.gemini/antigravity-cli`)
/// for top-level conversations, reusing unchanged transcript digests from
/// `old` and recording the new generation in `next`.
pub(crate) fn scan_antigravity(
    root: &Path,
    old: &ScanCache,
    next: &mut ScanCache,
) -> Vec<SessionRecord> {
    let db_path = root.join("conversation_summaries.db");
    if !db_path.is_file() {
        return Vec::new();
    }
    let rows = match read_summaries(&db_path) {
        Ok(rows) => rows,
        Err(error) => {
            warn!(path = %db_path.display(), %error, "antigravity summaries unreadable; skipping");
            return Vec::new();
        }
    };

    let brain_root = root.join("brain");
    let mut records = Vec::new();
    for row in rows {
        if !is_safe_id(&row.conversation_id) {
            continue;
        }
        let Some(cwd) = first_workspace_path(&row.workspace_uris) else {
            debug!(id = %row.conversation_id, "antigravity conversation has no workspace URI; skipped");
            continue;
        };
        let project_path = sidebar_key(&cwd);
        let transcript = brain_root
            .join(&row.conversation_id)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        let fallback_title = preferred_title(&row.title, &row.preview);
        let digest = digest_for_row(&transcript, fallback_title, row.step_count, old, next);
        let modified = parse_sqlite_timestamp(&row.last_modified_time)
            .or_else(|| file_sig(&transcript).map(|sig| sig.mtime));
        records.push(SessionRecord {
            provider: Provider::Antigravity,
            session_id: row.conversation_id,
            project_path,
            digest,
            modified,
            transcript,
        });
    }
    records
}

fn digest_for_row(
    transcript: &Path,
    fallback_title: Option<&str>,
    step_count: u32,
    old: &ScanCache,
    next: &mut ScanCache,
) -> SessionDigest {
    let sig = file_sig(transcript);
    let parsed = match (sig, old.digests.get(transcript)) {
        (Some(sig), Some(hit)) if hit.sig == sig => hit.digest.clone(),
        (Some(_), _) => read_transcript(transcript)
            .and_then(|content| digest_antigravity_transcript(&content, fallback_title)),
        (None, _) => None,
    };
    if let Some(sig) = sig {
        next.digests.insert(
            transcript.to_owned(),
            CachedDigest {
                sig,
                digest: parsed.clone(),
            },
        );
    }
    match parsed {
        Some(mut digest) => {
            if let Some(title) = fallback_title
                && digest.ai_title.is_none()
            {
                digest.ai_title = Some(truncate_utf16(title, SUMMARY_MAX_UNITS));
            }
            if digest.summary.is_empty()
                && let Some(title) = fallback_title
            {
                digest.summary = truncate_utf16(title, SUMMARY_MAX_UNITS);
            }
            digest
        }
        None => SessionDigest {
            summary: fallback_title
                .map(|t| truncate_utf16(t, SUMMARY_MAX_UNITS))
                .unwrap_or_default(),
            message_count: step_count,
            ..SessionDigest::default()
        },
    }
}

fn read_summaries(db_path: &Path) -> Result<Vec<SummaryRow>, rusqlite::Error> {
    let uri = sqlite_readonly_uri(db_path);
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(uri, flags)?;
    let mut stmt = conn.prepare(
        "SELECT conversation_id, title, preview, step_count, last_modified_time, workspace_uris \
         FROM conversation_summaries \
         WHERE nesting_depth = 0",
    )?;
    let rows = stmt.query_map([], |row| {
        let step_count: i64 = row.get(3)?;
        Ok(SummaryRow {
            conversation_id: row.get(0)?,
            title: row.get(1)?,
            preview: row.get(2)?,
            step_count: u32::try_from(step_count).unwrap_or(0),
            last_modified_time: row.get(4)?,
            workspace_uris: row.get(5)?,
        })
    })?;
    Ok(rows.flatten().collect())
}

/// Build an RFC-8089-style `file:` URI for SQLite `immutable=1` read-only
/// access so WAL databases owned by a running `agy` process are never mutated
/// or locked.
fn sqlite_readonly_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(raw.len() + 32);
    encoded.push_str("file:");
    if !raw.starts_with('/') {
        encoded.push('/');
    }
    for byte in raw.bytes() {
        match byte {
            b'?' => encoded.push_str("%3F"),
            b'#' => encoded.push_str("%23"),
            b'%' => encoded.push_str("%25"),
            b' ' => encoded.push_str("%20"),
            _ => encoded.push(char::from(byte)),
        }
    }
    encoded.push_str("?mode=ro&immutable=1");
    encoded
}

fn preferred_title<'a>(title: &'a str, preview: &'a str) -> Option<&'a str> {
    let t = title.trim();
    if !t.is_empty() {
        return Some(t);
    }
    let p = preview.trim();
    if !p.is_empty() { Some(p) } else { None }
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Parse the first usable local filesystem path from `workspace_uris` (a JSON
/// array of `file://` URIs such as `["file:///Users/me/repo"]`).
pub(crate) fn first_workspace_path(raw: &str) -> Option<String> {
    let uris: Vec<String> = serde_json::from_str(raw).ok()?;
    uris.into_iter().find_map(|uri| file_uri_to_path(&uri))
}

fn file_uri_to_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest)?;
    if decoded.is_empty() {
        return None;
    }
    // Windows file URIs carry a leading slash before the drive letter:
    // `file:///C:/Users/...` -> `C:/Users/...`.
    let bytes = decoded.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        return Some(decoded[1..].to_owned());
    }
    Some(decoded)
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1])?;
            let lo = hex_val(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parse an Antigravity `transcript.jsonl` into a [`SessionDigest`].
///
/// Pure and tolerant of empty or corrupt lines. Extracts the first `USER_INPUT`
/// prompt (stripping `<USER_REQUEST>` wrappers) as `summary`, counts
/// `USER_INPUT` + `PLANNER_RESPONSE` steps as `message_count`, and indexes user
/// and planner text up to 8000 UTF-16 units.
#[must_use]
pub(crate) fn digest_antigravity_transcript(
    content: &str,
    fallback_title: Option<&str>,
) -> Option<SessionDigest> {
    let mut summary = String::new();
    let mut message_count: u32 = 0;
    let mut text_content = String::new();
    let mut text_units: usize = 0;
    let mut tail: Vec<String> = Vec::new();
    let mut saw_step = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let Some(step_type) = value.get("type").and_then(serde_json::Value::as_str) else {
            continue;
        };
        saw_step = true;
        match step_type {
            "USER_INPUT" => {
                message_count = message_count.saturating_add(1);
                if let Some(raw) = value.get("content").and_then(serde_json::Value::as_str) {
                    let clean = extract_user_request(raw);
                    if !clean.is_empty() {
                        if summary.is_empty() {
                            summary = truncate_utf16(&clean, SUMMARY_MAX_UNITS);
                        }
                        append_indexed(&mut text_content, &mut text_units, &clean);
                        push_tail(&mut tail, &clean);
                    }
                }
            }
            "PLANNER_RESPONSE" => {
                message_count = message_count.saturating_add(1);
                if let Some(raw) = value.get("content").and_then(serde_json::Value::as_str) {
                    let clean = collapse_whitespace(raw);
                    if !clean.is_empty() {
                        append_indexed(&mut text_content, &mut text_units, &clean);
                        push_tail(&mut tail, &clean);
                    }
                }
            }
            _ => {}
        }
    }

    if !saw_step && fallback_title.is_none() {
        return None;
    }
    if summary.is_empty()
        && let Some(title) = fallback_title
    {
        summary = truncate_utf16(title, SUMMARY_MAX_UNITS);
    }
    Some(SessionDigest {
        summary,
        message_count,
        text_content,
        ai_title: fallback_title.map(|t| truncate_utf16(t, SUMMARY_MAX_UNITS)),
        tail,
        ..SessionDigest::default()
    })
}

/// Extract the user prompt inside `<USER_REQUEST>...</USER_REQUEST>` when
/// present, falling back to the raw string otherwise.
fn extract_user_request(raw: &str) -> String {
    const OPEN: &str = "<USER_REQUEST>";
    const CLOSE: &str = "</USER_REQUEST>";
    let inner = match (raw.find(OPEN), raw.find(CLOSE)) {
        (Some(start), Some(end)) if start + OPEN.len() <= end => &raw[start + OPEN.len()..end],
        _ => raw,
    };
    collapse_whitespace(inner)
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn append_indexed(buf: &mut String, units: &mut usize, text: &str) {
    if *units >= TEXT_CONTENT_MAX_UNITS {
        return;
    }
    let slice = truncate_utf16(text, TEXT_SLICE_UNITS);
    if !buf.is_empty() {
        buf.push('\n');
        *units += 1;
    }
    *units += slice.encode_utf16().count();
    buf.push_str(&slice);
}

fn push_tail(tail: &mut Vec<String>, text: &str) {
    let line = truncate_utf16(text, SUMMARY_MAX_UNITS);
    if tail.len() == TAIL_LINES {
        tail.remove(0);
    }
    tail.push(line);
}

fn truncate_utf16(s: &str, max_units: usize) -> String {
    let mut out = String::new();
    let mut units = 0;
    for ch in s.chars() {
        let len = ch.len_utf16();
        if units + len > max_units {
            break;
        }
        out.push(ch);
        units += len;
    }
    out
}

/// Parse an ISO-8601 / SQLite UTC timestamp (`YYYY-MM-DD HH:MM:SS[.frac][+00:00|Z]`)
/// into a [`SystemTime`] without pulling in a date crate.
fn parse_sqlite_timestamp(raw: &str) -> Option<SystemTime> {
    let bytes = raw.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let year: i64 = raw.get(0..4)?.parse().ok()?;
    let month: i64 = raw.get(5..7)?.parse().ok()?;
    let day: i64 = raw.get(8..10)?.parse().ok()?;
    let hour: i64 = raw.get(11..13)?.parse().ok()?;
    let min: i64 = raw.get(14..16)?.parse().ok()?;
    let sec: i64 = raw.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&min)
        || !(0..=60).contains(&sec)
    {
        return None;
    }
    // Howard Hinnant's civil-from-days inverse (UTC).
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let total_secs = days * 86_400 + hour * 3600 + min * 60 + sec;
    let secs = u64::try_from(total_secs).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_mock_db(root: &Path) -> Connection {
        let conn = Connection::open(root.join("conversation_summaries.db")).expect("open db");
        conn.execute(
            "CREATE TABLE conversation_summaries (
                conversation_id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                step_count INTEGER NOT NULL DEFAULT 0,
                last_modified_time DATETIME NOT NULL,
                workspace_uris TEXT NOT NULL,
                nesting_depth INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .expect("create table");
        conn
    }

    #[test]
    fn parses_transcript_stripping_user_request_tags_and_skipping_corrupt_lines() {
        let jsonl = concat!(
            "{\"type\":\"USER_INPUT\",\"content\":\"<USER_REQUEST>\\nFix the bug\\n</USER_REQUEST>\\n<ADDITIONAL_METADATA>ignored</ADDITIONAL_METADATA>\"}\n",
            "not valid json at all\n",
            "\n",
            "{\"type\":\"PLANNER_RESPONSE\",\"content\":\"Looking into it now.\"}\n",
            "{\"type\":\"GENERIC\",\"content\":\"tool output\"}\n",
        );
        let digest = digest_antigravity_transcript(jsonl, Some("Bug Fix")).expect("digest");
        assert_eq!(digest.summary, "Fix the bug");
        assert_eq!(digest.ai_title.as_deref(), Some("Bug Fix"));
        assert_eq!(digest.message_count, 2);
        assert!(digest.text_content.contains("Fix the bug"));
        assert!(digest.text_content.contains("Looking into it now."));
        assert!(!digest.text_content.contains("ADDITIONAL_METADATA"));
    }

    #[test]
    fn parses_unix_and_windows_file_uris_with_percent_encoding() {
        assert_eq!(
            first_workspace_path(r#"["file:///Users/me/My%20Repo"]"#).as_deref(),
            Some("/Users/me/My Repo")
        );
        assert_eq!(
            first_workspace_path(r#"["file:///C:/Users/me/repo"]"#).as_deref(),
            Some("C:/Users/me/repo")
        );
        assert_eq!(first_workspace_path("[]"), None);
        assert_eq!(first_workspace_path("not json"), None);
    }

    #[test]
    fn scans_mock_antigravity_dir_skips_subagents_and_uses_cache() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path().join("my-repo");
        fs::create_dir_all(&repo).expect("mkdir repo");
        let repo_uri = format!("[\"file://{}\"]", repo.to_string_lossy().replace('\\', "/"));

        let conn = create_mock_db(tmp.path());
        conn.execute(
            "INSERT INTO conversation_summaries VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f9012",
                "Refactor scanner",
                "Refactor scanner",
                4i64,
                "2026-11-21 16:48:11.000000+00:00",
                &repo_uri,
                0i64,
            ),
        )
        .expect("insert top-level");
        conn.execute(
            "INSERT INTO conversation_summaries VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "subagent-id",
                "Subagent task",
                "",
                2i64,
                "2026-11-21 16:49:00+00:00",
                &repo_uri,
                1i64,
            ),
        )
        .expect("insert subagent");
        conn.execute(
            "INSERT INTO conversation_summaries VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "--bad-flag",
                "Malicious id",
                "",
                1i64,
                "2026-11-21 16:49:00+00:00",
                &repo_uri,
                0i64,
            ),
        )
        .expect("insert bad id");
        drop(conn);

        let log_dir = tmp
            .path()
            .join("brain")
            .join("0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f9012")
            .join(".system_generated")
            .join("logs");
        fs::create_dir_all(&log_dir).expect("mkdir logs");
        let transcript = log_dir.join("transcript.jsonl");
        fs::write(
            &transcript,
            "{\"type\":\"USER_INPUT\",\"content\":\"<USER_REQUEST>\\nHello agy\\n</USER_REQUEST>\"}\n",
        )
        .expect("write transcript");

        let old = ScanCache::default();
        let mut next = ScanCache::default();
        let records = scan_antigravity(tmp.path(), &old, &mut next);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].provider, Provider::Antigravity);
        assert_eq!(
            records[0].session_id,
            "0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f9012"
        );
        assert_eq!(records[0].digest.summary, "Hello agy");
        assert_eq!(
            records[0].digest.ai_title.as_deref(),
            Some("Refactor scanner")
        );
        assert!(next.digests.contains_key(&transcript));

        // Second scan reuses the cached digest when the file signature matches.
        let mut third = ScanCache::default();
        let records2 = scan_antigravity(tmp.path(), &next, &mut third);
        assert_eq!(records2.len(), 1);
        assert_eq!(records2[0].digest, records[0].digest);
    }

    #[test]
    fn missing_database_returns_empty_without_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut next = ScanCache::default();
        assert!(scan_antigravity(tmp.path(), &ScanCache::default(), &mut next).is_empty());
    }
}
