//! `termherd-mcp` — the control-surface MCP server binary.
//!
//! A standalone stdio process an in-app Claude session launches via its
//! `mcpServers` config. It speaks newline-delimited JSON-RPC: one message per
//! line in on stdin, one response per line out on stdout. All protocol logic
//! lives in the library ([`termherd_mcp`]); this is just the transport loop.
//!
//! stdout is reserved for protocol traffic, so nothing else is ever printed
//! there — diagnostics, if added later, must go to stderr.

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
        // restart; a missing or invalid file reads as "no options set".
        let settings = load_settings();
        if let Some(response) = termherd_mcp::handle_message(&message, &settings) {
            if writeln!(stdout, "{response}").is_err() {
                break;
            }
            if stdout.flush().is_err() {
                break;
            }
        }
    }
}

/// Read `~/.termherd/settings.json` into a JSON value, falling back to an empty
/// object when it is missing or unreadable — the read-only surface then reports
/// every option as unset rather than failing.
fn load_settings() -> Value {
    let Some(path) = settings_path() else {
        return json!({});
    };
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| json!({})),
        Err(_) => json!({}),
    }
}

/// `~/.termherd/settings.json` — the same file the GUI shell reads.
fn settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".termherd").join("settings.json"))
}
