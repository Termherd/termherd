//! termherd-mcp — the control-surface MCP server (`F-mcp-control-surface`, #90).
//!
//! First, limited draft: a **read-only** view of termherd's configuration,
//! spoken as MCP over stdio so an in-app Claude session can ask "what can I
//! configure?" without leaving the conversation. It exposes one tool,
//! [`list_options`](handle_message), and the option **schema** as a resource.
//!
//! Writes (`set_option`), the keymap/`keys` surface and the workspace
//! orchestration tools (open session, split, focus, …) are deliberately out of
//! this draft — they land once the scope is settled (see the issue's phasing).
//!
//! This module is the **pure** half: JSON-RPC dispatch ([`handle_message`]) and
//! option resolution ([`resolve_options`]) take the parsed request and the
//! parsed `settings.json` value and return data — no I/O, no globals — so the
//! protocol is unit-testable. The thin stdio loop lives in `main.rs`.

use serde_json::{Value, json};

/// The MCP protocol revision we answer with when the client does not pin one.
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
/// The URI the option schema is published under as an MCP resource.
pub const SCHEMA_URI: &str = "termherd://options/schema";

/// One configurable option exposed over the control surface. `pointer` is a
/// JSON Pointer (RFC 6901) into `settings.json`, so reading a value never needs
/// to know the concrete `Settings` struct — it stays a string contract.
pub struct OptionSpec {
    /// Stable id a future `set_option` would address (e.g. `"theme"`).
    pub id: &'static str,
    /// JSON Pointer to the value inside `settings.json`.
    pub pointer: &'static str,
    /// Human description, surfaced to the model.
    pub description: &'static str,
    /// Coarse value kind: `"enum"`, `"string"`, or `"array"`.
    pub kind: &'static str,
    /// Allowed values for an `enum`, else empty.
    pub choices: &'static [&'static str],
}

/// The option catalog — the single source of what the control surface exposes.
/// Kept small for this first draft; `keys` (the keymap overrides) and the
/// orchestration surface are deferred.
pub const OPTIONS: &[OptionSpec] = &[
    OptionSpec {
        id: "theme",
        pointer: "/theme",
        description: "GUI chrome theme (the terminal grid keeps its own colours).",
        kind: "enum",
        choices: &["dark", "light"],
    },
    OptionSpec {
        id: "shell.program",
        pointer: "/shell/program",
        description: "Program launched for each shell session; unset means the platform default login shell.",
        kind: "string",
        choices: &[],
    },
    OptionSpec {
        id: "shell.args",
        pointer: "/shell/args",
        description: "Arguments passed to the shell program.",
        kind: "array",
        choices: &[],
    },
];

/// Resolve every option against a parsed `settings.json`, pairing each spec with
/// the value currently set (or `null` when the key is absent / the file empty).
#[must_use]
pub fn resolve_options(settings: &Value) -> Vec<Value> {
    OPTIONS
        .iter()
        .map(|spec| {
            let value = settings
                .pointer(spec.pointer)
                .cloned()
                .unwrap_or(Value::Null);
            let mut option = json!({
                "id": spec.id,
                "description": spec.description,
                "type": spec.kind,
                "value": value,
            });
            if !spec.choices.is_empty() {
                option["choices"] = json!(spec.choices);
            }
            option
        })
        .collect()
}

/// The option schema published as a resource: the catalog without live values,
/// so a client can learn the shape of the config independently of any one
/// machine's settings.
#[must_use]
pub fn schema_resource() -> Value {
    let options: Vec<Value> = OPTIONS
        .iter()
        .map(|spec| {
            let mut option = json!({
                "id": spec.id,
                "description": spec.description,
                "type": spec.kind,
            });
            if !spec.choices.is_empty() {
                option["choices"] = json!(spec.choices);
            }
            option
        })
        .collect();
    json!({ "options": options })
}

/// Handle one decoded JSON-RPC message against the given `settings.json` value.
/// Returns the response to write back, or `None` for a notification (a message
/// with no `id`, e.g. `notifications/initialized`) which JSON-RPC never answers.
#[must_use]
pub fn handle_message(message: &Value, settings: &Value) -> Option<Value> {
    // A message without an `id` is a notification: act if needed, never reply.
    let id = message.get("id")?.clone();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let outcome = match method {
        "initialize" => Ok(initialize_result(message)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => tool_call_result(message, settings),
        "resources/list" => Ok(resources_list_result()),
        "resources/read" => resource_read_result(message),
        other => Err(error_object(-32601, &format!("method not found: {other}"))),
    };

    Some(match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    })
}

/// The `initialize` handshake result, echoing the client's requested protocol
/// version when it pins one so we never force a downgrade.
fn initialize_result(message: &Value) -> Value {
    let version = message
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {}, "resources": {} },
        "serverInfo": { "name": "termherd-mcp", "version": env!("CARGO_PKG_VERSION") },
    })
}

/// The single tool this draft exposes: `list_options`, which takes no arguments.
fn tools_list_result() -> Value {
    json!({
        "tools": [{
            "name": "list_options",
            "description": "List termherd's configurable options with their current values.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        }],
    })
}

/// Dispatch a `tools/call`. Only `list_options` is known in this draft.
fn tool_call_result(message: &Value, settings: &Value) -> Result<Value, Value> {
    let name = message
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match name {
        "list_options" => {
            let options = Value::Array(resolve_options(settings));
            let text = serde_json::to_string_pretty(&options).unwrap_or_default();
            Ok(json!({
                "content": [{ "type": "text", "text": text }],
                "isError": false,
            }))
        }
        other => Err(error_object(-32602, &format!("unknown tool: {other}"))),
    }
}

/// Advertise the one resource: the option schema.
fn resources_list_result() -> Value {
    json!({
        "resources": [{
            "uri": SCHEMA_URI,
            "name": "Option schema",
            "description": "The schema of termherd's configurable options.",
            "mimeType": "application/json",
        }],
    })
}

/// Serve a `resources/read` for the schema URI; any other URI is an error.
fn resource_read_result(message: &Value) -> Result<Value, Value> {
    let uri = message
        .pointer("/params/uri")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if uri != SCHEMA_URI {
        return Err(error_object(-32602, &format!("unknown resource: {uri}")));
    }
    let text = serde_json::to_string_pretty(&schema_resource()).unwrap_or_default();
    Ok(json!({
        "contents": [{
            "uri": SCHEMA_URI,
            "mimeType": "application/json",
            "text": text,
        }],
    }))
}

/// A JSON-RPC error object.
fn error_object(code: i64, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Value {
        json!({ "theme": "light", "shell": { "program": "pwsh", "args": ["-NoLogo"] } })
    }

    #[test]
    fn resolve_options_pairs_specs_with_current_values() {
        let opts = resolve_options(&settings());
        let theme = opts.iter().find(|o| o["id"] == "theme").expect("theme");
        assert_eq!(theme["value"], json!("light"));
        assert_eq!(theme["choices"], json!(["dark", "light"]));
        let program = opts
            .iter()
            .find(|o| o["id"] == "shell.program")
            .expect("shell.program");
        assert_eq!(program["value"], json!("pwsh"));
    }

    #[test]
    fn absent_settings_resolve_to_null_values() {
        let opts = resolve_options(&json!({}));
        assert!(
            opts.iter().all(|o| o["value"] == Value::Null),
            "an empty settings file leaves every option unset"
        );
        // Every catalog entry is still listed, so the model sees what exists.
        assert_eq!(opts.len(), OPTIONS.len());
    }

    #[test]
    fn initialize_echoes_the_clients_protocol_version() {
        let req = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18" }
        });
        let resp = handle_message(&req, &json!({})).expect("a response");
        assert_eq!(resp["result"]["protocolVersion"], json!("2025-06-18"));
        assert_eq!(resp["result"]["serverInfo"]["name"], json!("termherd-mcp"));
        assert!(resp["result"]["capabilities"].get("tools").is_some());
    }

    #[test]
    fn initialize_without_a_version_falls_back_to_the_default() {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let resp = handle_message(&req, &json!({})).expect("a response");
        assert_eq!(
            resp["result"]["protocolVersion"],
            json!(DEFAULT_PROTOCOL_VERSION)
        );
    }

    #[test]
    fn tools_list_advertises_list_options() {
        let req = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let resp = handle_message(&req, &json!({})).expect("a response");
        let tools = resp["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], json!("list_options"));
    }

    #[test]
    fn tools_call_list_options_returns_the_current_config() {
        let req = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "list_options", "arguments": {} }
        });
        let resp = handle_message(&req, &settings()).expect("a response");
        assert_eq!(resp["result"]["isError"], json!(false));
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("text content");
        let parsed: Value = serde_json::from_str(text).expect("the payload is JSON");
        let theme = parsed
            .as_array()
            .and_then(|a| a.iter().find(|o| o["id"] == "theme"))
            .expect("theme option");
        assert_eq!(theme["value"], json!("light"));
    }

    #[test]
    fn an_unknown_tool_is_an_error_not_a_panic() {
        let req = json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "set_option", "arguments": {} }
        });
        let resp = handle_message(&req, &json!({})).expect("a response");
        assert_eq!(resp["error"]["code"], json!(-32602));
    }

    #[test]
    fn resources_read_serves_the_schema_and_rejects_other_uris() {
        let ok = json!({
            "jsonrpc": "2.0", "id": 5, "method": "resources/read",
            "params": { "uri": SCHEMA_URI }
        });
        let resp = handle_message(&ok, &json!({})).expect("a response");
        let text = resp["result"]["contents"][0]["text"]
            .as_str()
            .expect("schema text");
        let schema: Value = serde_json::from_str(text).expect("schema is JSON");
        assert!(schema["options"].is_array());

        let bad = json!({
            "jsonrpc": "2.0", "id": 6, "method": "resources/read",
            "params": { "uri": "termherd://nope" }
        });
        let resp = handle_message(&bad, &json!({})).expect("a response");
        assert_eq!(resp["error"]["code"], json!(-32602));
    }

    #[test]
    fn an_unknown_method_returns_method_not_found() {
        let req = json!({ "jsonrpc": "2.0", "id": 7, "method": "frobnicate" });
        let resp = handle_message(&req, &json!({})).expect("a response");
        assert_eq!(resp["error"]["code"], json!(-32601));
    }

    #[test]
    fn a_notification_yields_no_response() {
        // No `id` → JSON-RPC notification; we must not reply.
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle_message(&note, &json!({})).is_none());
    }
}
