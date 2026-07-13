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
        // restart; a missing file reads as "no options set". `malformed` marks a
        // file that exists but does not parse — safe to read as empty, but a
        // write must refuse it rather than clobber the user's content.
        let (settings, malformed) = read_settings();
        let reply = termherd_mcp::handle_message(&message, &settings);
        // Perform the write effect the pure layer described, before replying, so
        // a caller that reads back sees its own change. Any write failure — or a
        // refusal to overwrite a malformed file — becomes a JSON-RPC error for
        // the same request rather than a false success (the silent-catch this
        // project exists to avoid).
        let mut response = reply.response;
        if let Some(new_settings) = reply.write_settings {
            let outcome = if malformed {
                Err(
                    "settings file exists but is not valid JSON; refusing to overwrite it"
                        .to_owned(),
                )
            } else {
                store_settings(&new_settings)
            };
            if let Err(err) = outcome {
                eprintln!("termherd-mcp: {err}");
                response = write_error_response(&message, &err);
            }
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

/// Read `~/.termherd/settings.json` into a JSON value plus a `malformed` flag.
/// A missing or unreadable file reads as an empty object (`malformed = false`) —
/// the read surface then reports every option as unset. A file that exists but
/// does not parse reads as empty too, but with `malformed = true`, so the write
/// path can refuse to overwrite it rather than discard the user's content.
fn read_settings() -> (Value, bool) {
    let Some(path) = settings_path() else {
        return (json!({}), false);
    };
    match std::fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(value) => (value, false),
            Err(_) => (json!({}), true),
        },
        Err(_) => (json!({}), false),
    }
}

/// Persist the mutated settings to `~/.termherd/settings.json`, pretty-printed
/// like the file the GUI writes. Creates the `~/.termherd` directory if needed
/// and replaces the file **atomically** — write a sibling temp file, then rename
/// — so a crash or a concurrent reader never sees a half-written file. Returns an
/// error message (not a panic — the server stays up) so the caller can report the
/// failure instead of a false success.
fn store_settings(settings: &Value) -> Result<(), String> {
    let path = settings_path().ok_or("no home directory; cannot write settings")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("could not create {}: {err}", dir.display()))?;
    }
    let raw = serde_json::to_string_pretty(settings)
        .map_err(|err| format!("could not encode settings: {err}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, raw + "\n")
        .map_err(|err| format!("could not write {}: {err}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|err| format!("could not replace {}: {err}", path.display()))
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
