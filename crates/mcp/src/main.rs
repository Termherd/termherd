//! `termherd-mcp` — the control-surface MCP server binary.
//!
//! A standalone stdio process an in-app Claude session launches via its
//! `mcpServers` config. It speaks newline-delimited JSON-RPC: one message per
//! line in on stdin, one response per line out on stdout. All protocol logic
//! lives in the library ([`termherd_mcp`]); this is the transport loop and the
//! two I/O effects — reading and writing `settings.json`.
//!
//! stdout is reserved for protocol traffic, so nothing else is ever printed
//! there — diagnostics go to stderr.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde_json::{Value, json};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // A line that is not valid JSON-RPC is skipped: with no decodable id we
        // cannot address an error response to it.
        let Ok(message) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        // Read settings fresh per request so a live edit is reflected without a
        // restart; a missing file reads as "no options set".
        let (settings, writable) = load_settings();
        let reply = termherd_mcp::handle_message(&message, &settings);
        // Perform the write effect the pure layer described, before replying,
        // so a caller that reads back sees its own change. A write that fails
        // must not report success — turn it into a JSON-RPC error for the same
        // request rather than swallow it (the very silent-catch this project
        // exists to avoid).
        let mut response = reply.response;
        if let Some(new_settings) = reply.write_settings
            && let Err(err) = writable.and_then(|()| store_settings(&new_settings))
        {
            eprintln!("termherd-mcp: {err}");
            response = write_error_response(&message, &err);
        }
        if let Some(response) = response {
            if writeln!(stdout, "{response}").is_err() {
                break;
            }
            if stdout.flush().is_err() {
                break;
            }
        }
    }
}

/// A JSON-RPC error response reporting a failed settings write, addressed to the
/// request's `id`. `None` when the message carried no `id` (a notification never
/// stages a write, so this is defensive) — nothing to answer.
fn write_error_response(message: &Value, detail: &str) -> Option<Value> {
    let id = message.get("id")?;
    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32000, "message": format!("could not persist settings: {detail}") },
    }))
}

/// Read `~/.termherd/settings.json`, falling back to an empty object when it
/// cannot be used — `list_options` then reports every option as unset rather
/// than failing. The second half says whether a write may replace the file:
/// not when it exists but is not a JSON object (see [`termherd_mcp::file`]).
fn load_settings() -> (Value, Result<(), String>) {
    match settings_path() {
        Some(path) => load_settings_at(&path),
        None => (json!({}), Ok(())),
    }
}

/// The body of [`load_settings`], on an explicit path.
fn load_settings_at(path: &std::path::Path) -> (Value, Result<(), String>) {
    match termherd_mcp::file::read_object(path) {
        Ok(value) => (value, Ok(())),
        Err(reason) => (json!({}), Err(reason)),
    }
}

/// Persist the mutated settings to `~/.termherd/settings.json`. Returns an
/// error message (not a panic — the server stays up) so the caller can report
/// the failure instead of a false success.
fn store_settings(settings: &Value) -> Result<(), String> {
    let path = settings_path().ok_or("no home directory; cannot write settings")?;
    termherd_mcp::file::write(&path, settings)
}

/// `~/.termherd/settings.json` — the same file the GUI shell reads.
fn settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".termherd").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_does_not_parse_is_read_as_empty_but_never_written_over() {
        let dir = std::env::temp_dir().join(format!("termherd-mcp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("settings.json");

        std::fs::write(&path, "{ broken").expect("write");
        let (settings, writable) = load_settings_at(&path);
        assert_eq!(settings, json!({}));
        assert!(writable.is_err(), "a write would discard the user's file");

        std::fs::write(&path, r#"{ "theme": "dark" }"#).expect("write");
        assert_eq!(
            load_settings_at(&path),
            (json!({ "theme": "dark" }), Ok(()))
        );

        std::fs::remove_file(&path).expect("remove");
        assert_eq!(
            load_settings_at(&path),
            (json!({}), Ok(())),
            "missing is writable"
        );
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_failed_write_becomes_an_error_for_the_same_request() {
        let req = json!({
            "jsonrpc": "2.0", "id": 42, "method": "tools/call",
            "params": { "name": "set_option", "arguments": { "id": "theme", "value": "dark" } }
        });
        let resp = write_error_response(&req, "disk full").expect("an error response");
        // Addressed to the caller's id, and a real error — never a false success.
        assert_eq!(resp["id"], json!(42));
        assert_eq!(resp["error"]["code"], json!(-32000));
        assert!(
            resp["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("disk full")),
            "the failure detail reaches the caller"
        );
    }

    #[test]
    fn a_notification_has_no_error_response() {
        // No `id` → nothing to answer, even on a (defensive) write failure.
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(write_error_response(&note, "whatever").is_none());
    }
}
