//! termherd-mcp — the control-surface MCP server (`F-mcp-control-surface`).
//!
//! A view of termherd's configuration, spoken as MCP over stdio so an in-app
//! Claude session can ask "what can I configure?" — and now change it — without
//! leaving the conversation. It exposes the [`list_options`](handle_message)
//! and [`set_option`](handle_message) tools, the option **schema** resource,
//! and the keymap **`keys`** catalogue as a resource.
//!
//! Live workspace orchestration (open session, split, focus, …) needs the live
//! transport and stays out of this stdio slice — those land as separate
//! children of the control-surface split.
//!
//! This module is the **pure** half: JSON-RPC dispatch ([`handle_message`]),
//! option resolution ([`resolve_options`]) and the [`set_option`] mutation take
//! the parsed request and the parsed `settings.json` value and return data —
//! no I/O, no globals — so the protocol is unit-testable. A write is *described*
//! (the returned [`Reply::write_settings`]) here and *performed* by the thin
//! stdio loop in `main.rs`, keeping the same Event→Effect split as `core`.

use serde_json::{Value, json};

/// The MCP protocol revision we answer with when the client does not pin one.
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
/// The URI the option schema is published under as an MCP resource.
pub const SCHEMA_URI: &str = "termherd://options/schema";
/// The URI the keymap (`keys`) catalogue is published under as an MCP resource.
pub const KEYS_URI: &str = "termherd://keys/schema";

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
    OptionSpec {
        id: "terminal.colors.scheme",
        pointer: "/terminal/colors/scheme",
        description: "Built-in terminal colour scheme the overrides start from; unset means the built-in default.",
        kind: "enum",
        choices: &[
            "solarized-dark",
            "solarized-light",
            "gruvbox-dark",
            "gruvbox-light",
        ],
    },
    OptionSpec {
        id: "terminal.colors.foreground",
        pointer: "/terminal/colors/foreground",
        description: "Terminal default text colour, \"#rrggbb\"; unset means the built-in scheme.",
        kind: "string",
        choices: &[],
    },
    OptionSpec {
        id: "terminal.colors.background",
        pointer: "/terminal/colors/background",
        description: "Terminal background colour, \"#rrggbb\"; unset means the built-in scheme.",
        kind: "string",
        choices: &[],
    },
    OptionSpec {
        id: "terminal.colors.cursor",
        pointer: "/terminal/colors/cursor",
        description: "Terminal cursor block colour, \"#rrggbb\"; unset means the built-in scheme.",
        kind: "string",
        choices: &[],
    },
    OptionSpec {
        id: "terminal.colors.palette",
        pointer: "/terminal/colors/palette",
        description: "The 16 ANSI terminal colours (normal 0-7, bright 8-15), each \"#rrggbb\".",
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

/// The outcome of a [`set_option`]: either the option was applied (carrying the
/// settings value to persist) or the value was refused. A value that does not
/// match the option's declared shape is **rejected, not written** — the GUI
/// parses `settings.json` as one struct and resets *every* setting to its
/// default on a single unparseable field, so persisting a bad value would
/// silently wipe the user's whole config, not degrade one field. An unknown id
/// is signalled by `None` from [`set_option`] (no pointer to address).
pub enum SetOutcome {
    /// The option was set; carries the full settings value to persist.
    Applied(Value),
    /// The value was refused (it would corrupt the file the GUI must parse).
    /// Carries the reason, for the caller's error.
    Rejected(String),
}

/// Apply `set_option` purely: resolve `id` against [`OPTIONS`], and either apply
/// the value or reject it. A value that violates the option's kind/choices is
/// [`SetOutcome::Rejected`] with no write. `null` unsets the option by *removing*
/// its key (so the field falls back to its default) rather than persisting an
/// explicit `null` the GUI's non-optional fields cannot parse. Returns `None`
/// for an unknown `id` — the "broken invariant → `None`" idiom in `core`.
#[must_use]
pub fn set_option(settings: &Value, id: &str, value: &Value) -> Option<SetOutcome> {
    let spec = OPTIONS.iter().find(|spec| spec.id == id)?;
    if let Some(reason) = shape_violation(spec, value) {
        return Some(SetOutcome::Rejected(reason));
    }
    let mut new_settings = settings.clone();
    if value.is_null() {
        remove_pointer(&mut new_settings, spec.pointer);
    } else {
        set_pointer(&mut new_settings, spec.pointer, value.clone());
    }
    Some(SetOutcome::Applied(new_settings))
}

/// The reason `value` violates `spec`'s declared shape, or `None` when it fits.
/// `null` always fits — it unsets the option. Otherwise an `enum` value must be
/// one of the declared choices, a `string` must be a JSON string, and an `array`
/// must be a JSON array; each is a shape the GUI's typed `Settings` requires, so
/// a mismatch must be refused before it can corrupt the file.
fn shape_violation(spec: &OptionSpec, value: &Value) -> Option<String> {
    if value.is_null() {
        return None;
    }
    match spec.kind {
        "enum" => value
            .as_str()
            .is_none_or(|s| !spec.choices.contains(&s))
            .then(|| {
                format!(
                    "value {value} is not one of the allowed choices for {}: {}",
                    spec.id,
                    spec.choices.join(", ")
                )
            }),
        "string" => (!value.is_string())
            .then(|| format!("option {} expects a string, got {value}", spec.id)),
        "array" => {
            (!value.is_array()).then(|| format!("option {} expects an array, got {value}", spec.id))
        }
        _ => None,
    }
}

/// Set `value` at a JSON Pointer (RFC 6901) in `root`, creating intermediate
/// objects for any missing path segments (`serde_json::pointer_mut` only walks
/// existing ones). A non-object met mid-path is replaced with an object, so a
/// well-formed catalog pointer — all escape-free (`/terminal/colors/…`) — always
/// lands. An empty pointer (the whole document) is a no-op; the catalog has none.
fn set_pointer(root: &mut Value, pointer: &str, value: Value) {
    let Some(path) = pointer.strip_prefix('/') else {
        return;
    };
    let segments: Vec<&str> = path.split('/').collect();
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let mut cursor = root;
    for segment in parents {
        if !cursor.is_object() {
            *cursor = json!({});
        }
        let Some(map) = cursor.as_object_mut() else {
            return;
        };
        cursor = map.entry((*segment).to_owned()).or_insert(Value::Null);
    }
    if !cursor.is_object() {
        *cursor = json!({});
    }
    if let Some(map) = cursor.as_object_mut() {
        map.insert((*last).to_owned(), value);
    }
}

/// Remove the key a JSON Pointer addresses, walking only existing objects. Used
/// to unset an option so its field falls back to the default — persisting an
/// explicit `null` would instead fail the GUI's non-optional field parse. A path
/// whose parent is absent or not an object is a no-op (nothing to remove).
fn remove_pointer(root: &mut Value, pointer: &str) {
    let Some(path) = pointer.strip_prefix('/') else {
        return;
    };
    let segments: Vec<&str> = path.split('/').collect();
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let mut cursor = root;
    for segment in parents {
        match cursor.as_object_mut().and_then(|map| map.get_mut(*segment)) {
            Some(next) => cursor = next,
            None => return,
        }
    }
    if let Some(map) = cursor.as_object_mut() {
        map.remove(*last);
    }
}

/// The keymap catalogue served as the `keys` resource: every bindable action
/// with its default chords and the chord(s) currently bound in `settings.json`
/// (`null` when unbound). Read-only for now — rebinding is a later slice.
#[must_use]
pub fn keys_resource(settings: &Value) -> Value {
    let actions: Vec<Value> = termherd_core::action_catalog()
        .into_iter()
        .map(|binding| {
            // Current binding: whatever the `keys` section holds for this action
            // (a string or an array of chords), or `null` when unbound.
            let current = settings
                .pointer(&format!("/keys/{}", binding.name))
                .cloned()
                .unwrap_or(Value::Null);
            json!({
                "name": binding.name,
                "default": binding.default_chords,
                "current": current,
            })
        })
        .collect();
    json!({ "actions": actions })
}

/// What one decoded JSON-RPC message resolves to: the response to write back
/// (`None` for a notification) and, optionally, a `settings.json` mutation the
/// transport must persist. Keeps `handle_message` pure — it *describes* the
/// write; `main.rs` performs it (the same Event→Effect split as `core`).
#[derive(Debug, Default)]
pub struct Reply {
    /// The JSON-RPC response, or `None` for a notification (no `id`).
    pub response: Option<Value>,
    /// A `settings.json` value to persist, or `None` when nothing changed.
    pub write_settings: Option<Value>,
}

/// Handle one decoded JSON-RPC message against the given `settings.json` value.
/// Returns a [`Reply`]: the response to write back plus any settings mutation to
/// persist. A message with no `id` is a notification — no response, no write.
#[must_use]
pub fn handle_message(message: &Value, settings: &Value) -> Reply {
    // A message without an `id` is a notification: act if needed, never reply.
    let Some(id) = message.get("id").cloned() else {
        return Reply::default();
    };
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();

    // A tool call may carry a settings mutation to persist alongside its result.
    let mut write_settings = None;
    let outcome = match method {
        "initialize" => Ok(initialize_result(message)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => tool_call_result(message, settings, &mut write_settings),
        "resources/list" => Ok(resources_list_result()),
        "resources/read" => resource_read_result(message, settings),
        other => Err(error_object(-32601, &format!("method not found: {other}"))),
    };

    let response = match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    };
    Reply {
        response: Some(response),
        write_settings,
    }
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

/// The tools this slice exposes: `list_options` (no arguments) and `set_option`
/// (an option `id` plus a new `value`).
fn tools_list_result() -> Value {
    json!({
        "tools": [
            {
                "name": "list_options",
                "description": "List termherd's configurable options with their current values.",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            },
            {
                "name": "set_option",
                "description": "Set one termherd option by id; the change lands in settings.json and applies on restart.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "The option id, e.g. \"theme\"." },
                        "value": { "description": "The new value (type per the option's schema)." },
                    },
                    "required": ["id", "value"],
                    "additionalProperties": false,
                },
            },
        ],
    })
}

/// Dispatch a `tools/call`. `list_options` reads; `set_option` resolves the
/// mutation and, on success, records the settings to persist in `write_settings`
/// so the transport can write them.
fn tool_call_result(
    message: &Value,
    settings: &Value,
    write_settings: &mut Option<Value>,
) -> Result<Value, Value> {
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
        "set_option" => set_option_call(message, settings, write_settings),
        other => Err(error_object(-32602, &format!("unknown tool: {other}"))),
    }
}

/// The `set_option` tool call: read `id`/`value` from the arguments, apply the
/// pure mutation, and — when the value is accepted — stage the new settings for
/// the transport to persist. Errors (no write) for an unknown `id`, a missing
/// `value` (required — a missing one must not silently unset), or a value the
/// option's shape refuses.
fn set_option_call(
    message: &Value,
    settings: &Value,
    write_settings: &mut Option<Value>,
) -> Result<Value, Value> {
    let id = message
        .pointer("/params/arguments/id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    // `value` is required. A missing key must error, not default to null — that
    // would silently unset the option (and an explicit null is a deliberate
    // unset). An explicit `"value": null` in the call still reaches here as Null.
    let Some(value) = message.pointer("/params/arguments/value").cloned() else {
        return Err(error_object(-32602, "missing required argument: value"));
    };

    match set_option(settings, id, &value) {
        None => Err(error_object(-32602, &format!("unknown option: {id}"))),
        Some(SetOutcome::Rejected(reason)) => Err(error_object(-32602, &reason)),
        Some(SetOutcome::Applied(new_settings)) => {
            *write_settings = Some(new_settings);
            Ok(json!({
                "content": [{ "type": "text", "text": format!("set {id}") }],
                "isError": false,
            }))
        }
    }
}

/// Advertise the resources: the option schema and the keymap (`keys`) catalogue.
fn resources_list_result() -> Value {
    json!({
        "resources": [
            {
                "uri": SCHEMA_URI,
                "name": "Option schema",
                "description": "The schema of termherd's configurable options.",
                "mimeType": "application/json",
            },
            {
                "uri": KEYS_URI,
                "name": "Keymap catalogue",
                "description": "The bindable actions with their default and current key chords.",
                "mimeType": "application/json",
            },
        ],
    })
}

/// Serve a `resources/read` for the schema or `keys` URI; any other URI is an
/// error. The `keys` payload reflects the current `settings.json` bindings.
fn resource_read_result(message: &Value, settings: &Value) -> Result<Value, Value> {
    let uri = message
        .pointer("/params/uri")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let body = match uri {
        SCHEMA_URI => schema_resource(),
        KEYS_URI => keys_resource(settings),
        other => return Err(error_object(-32602, &format!("unknown resource: {other}"))),
    };
    let text = serde_json::to_string_pretty(&body).unwrap_or_default();
    Ok(json!({
        "contents": [{
            "uri": uri,
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

    /// Drive `handle_message` and unwrap its response — for tests that only care
    /// about the reply, not the write side.
    fn respond(message: &Value, settings: &Value) -> Value {
        handle_message(message, settings)
            .response
            .expect("a response")
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
        let resp = respond(&req, &json!({}));
        assert_eq!(resp["result"]["protocolVersion"], json!("2025-06-18"));
        assert_eq!(resp["result"]["serverInfo"]["name"], json!("termherd-mcp"));
        assert!(resp["result"]["capabilities"].get("tools").is_some());
    }

    #[test]
    fn initialize_without_a_version_falls_back_to_the_default() {
        let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let resp = respond(&req, &json!({}));
        assert_eq!(
            resp["result"]["protocolVersion"],
            json!(DEFAULT_PROTOCOL_VERSION)
        );
    }

    #[test]
    fn tools_list_advertises_read_and_write_tools() {
        let req = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let resp = respond(&req, &json!({}));
        let tools = resp["result"]["tools"].as_array().expect("tools array");
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"list_options"), "reads stay advertised");
        assert!(
            names.contains(&"set_option"),
            "the write tool is advertised"
        );
        // `set_option` declares its required arguments so the model calls it right.
        let set = tools
            .iter()
            .find(|t| t["name"] == "set_option")
            .expect("set_option tool");
        assert_eq!(set["inputSchema"]["required"], json!(["id", "value"]));
    }

    #[test]
    fn tools_call_list_options_returns_the_current_config() {
        let req = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "list_options", "arguments": {} }
        });
        let resp = respond(&req, &settings());
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
            "params": { "name": "frobnicate", "arguments": {} }
        });
        let resp = respond(&req, &json!({}));
        assert_eq!(resp["error"]["code"], json!(-32602));
    }

    #[test]
    fn resources_read_serves_the_schema_and_rejects_other_uris() {
        let ok = json!({
            "jsonrpc": "2.0", "id": 5, "method": "resources/read",
            "params": { "uri": SCHEMA_URI }
        });
        let resp = respond(&ok, &json!({}));
        let text = resp["result"]["contents"][0]["text"]
            .as_str()
            .expect("schema text");
        let schema: Value = serde_json::from_str(text).expect("schema is JSON");
        assert!(schema["options"].is_array());

        let bad = json!({
            "jsonrpc": "2.0", "id": 6, "method": "resources/read",
            "params": { "uri": "termherd://nope" }
        });
        let resp = respond(&bad, &json!({}));
        assert_eq!(resp["error"]["code"], json!(-32602));
    }

    #[test]
    fn an_unknown_method_returns_method_not_found() {
        let req = json!({ "jsonrpc": "2.0", "id": 7, "method": "frobnicate" });
        let resp = respond(&req, &json!({}));
        assert_eq!(resp["error"]["code"], json!(-32601));
    }

    #[test]
    fn a_notification_yields_no_response_and_no_write() {
        // No `id` → JSON-RPC notification; we must neither reply nor write.
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let reply = handle_message(&note, &json!({}));
        assert!(reply.response.is_none());
        assert!(reply.write_settings.is_none());
    }

    // --- set_option (the write half) --------------------------------------

    /// Unwrap an `Applied` outcome to its settings, failing on `Rejected`.
    fn applied(outcome: Option<SetOutcome>) -> Value {
        match outcome.expect("known option") {
            SetOutcome::Applied(settings) => settings,
            SetOutcome::Rejected(reason) => panic!("expected Applied, got Rejected: {reason}"),
        }
    }

    #[test]
    fn set_option_sets_a_known_option_value() {
        let out = applied(set_option(&settings(), "theme", &json!("dark")));
        assert_eq!(out.pointer("/theme"), Some(&json!("dark")));
    }

    #[test]
    fn set_option_creates_missing_parent_objects() {
        // `terminal.colors.background` is nested; none of it exists yet.
        let out = applied(set_option(
            &json!({}),
            "terminal.colors.background",
            &json!("#101010"),
        ));
        assert_eq!(
            out.pointer("/terminal/colors/background"),
            Some(&json!("#101010"))
        );
    }

    #[test]
    fn set_option_leaves_sibling_config_untouched() {
        let out = applied(set_option(&settings(), "theme", &json!("dark")));
        // The shell block set in `settings()` must survive the theme write.
        assert_eq!(out.pointer("/shell/program"), Some(&json!("pwsh")));
    }

    #[test]
    fn set_option_on_an_unknown_id_is_none() {
        assert!(set_option(&settings(), "no.such.option", &json!("x")).is_none());
    }

    #[test]
    fn set_option_rejects_an_out_of_choices_enum() {
        // "banana" is not a theme choice. Persisting it would make the GUI fail
        // to parse the whole file and reset every setting — so it is refused, not
        // written.
        let outcome = set_option(&settings(), "theme", &json!("banana")).expect("known option");
        assert!(
            matches!(outcome, SetOutcome::Rejected(_)),
            "an out-of-choices enum is rejected"
        );
    }

    #[test]
    fn set_option_rejects_a_type_mismatch() {
        // `shell.args` is an array; a string is the wrong shape and would break
        // the GUI's typed parse, so it is refused.
        let outcome =
            set_option(&settings(), "shell.args", &json!("not-an-array")).expect("known option");
        assert!(
            matches!(outcome, SetOutcome::Rejected(_)),
            "a wrong-typed value is rejected"
        );
    }

    #[test]
    fn set_option_null_unsets_by_removing_the_key() {
        // Unsetting must remove the key (fall back to default), not persist an
        // explicit null the GUI's non-optional theme field cannot parse.
        let out = applied(set_option(&settings(), "theme", &Value::Null));
        assert_eq!(out.pointer("/theme"), None, "the key is gone, not null");
        // A sibling stays put — only the one key is removed.
        assert_eq!(out.pointer("/shell/program"), Some(&json!("pwsh")));
    }

    #[test]
    fn tools_call_set_option_stages_the_write() {
        let req = json!({
            "jsonrpc": "2.0", "id": 8, "method": "tools/call",
            "params": { "name": "set_option", "arguments": { "id": "theme", "value": "dark" } }
        });
        let reply = handle_message(&req, &settings());
        let resp = reply.response.expect("a response");
        assert_eq!(resp["result"]["isError"], json!(false));
        // The transport is handed the mutated settings to persist.
        let write = reply.write_settings.expect("a staged write");
        assert_eq!(write.pointer("/theme"), Some(&json!("dark")));
    }

    #[test]
    fn tools_call_set_option_unknown_id_errors_and_stages_no_write() {
        let req = json!({
            "jsonrpc": "2.0", "id": 9, "method": "tools/call",
            "params": { "name": "set_option", "arguments": { "id": "nope", "value": 1 } }
        });
        let reply = handle_message(&req, &settings());
        let resp = reply.response.expect("a response");
        assert_eq!(resp["error"]["code"], json!(-32602));
        assert!(
            reply.write_settings.is_none(),
            "an unknown id writes nothing"
        );
    }

    #[test]
    fn tools_call_set_option_rejects_invalid_value_without_writing() {
        let req = json!({
            "jsonrpc": "2.0", "id": 12, "method": "tools/call",
            "params": { "name": "set_option", "arguments": { "id": "theme", "value": "banana" } }
        });
        let reply = handle_message(&req, &settings());
        let resp = reply.response.expect("a response");
        assert_eq!(resp["error"]["code"], json!(-32602));
        assert!(
            reply.write_settings.is_none(),
            "a rejected value writes nothing — the file is never corrupted"
        );
    }

    #[test]
    fn tools_call_set_option_missing_value_errors_without_writing() {
        // `value` is required; a call that omits it must error, not silently
        // unset the option.
        let req = json!({
            "jsonrpc": "2.0", "id": 13, "method": "tools/call",
            "params": { "name": "set_option", "arguments": { "id": "theme" } }
        });
        let reply = handle_message(&req, &settings());
        let resp = reply.response.expect("a response");
        assert_eq!(resp["error"]["code"], json!(-32602));
        assert!(
            reply.write_settings.is_none(),
            "a missing value writes nothing"
        );
    }

    // --- keys resource (the read-only keymap catalogue) -------------------

    #[test]
    fn keys_resource_lists_the_action_catalogue_with_current_bindings() {
        let cfg = json!({ "keys": { "copy": "ctrl+y" } });
        let keys = keys_resource(&cfg);
        let actions = keys["actions"].as_array().expect("actions array");
        // The catalogue mirrors core's bindable vocabulary, not an empty list.
        assert_eq!(actions.len(), termherd_core::action_catalog().len());
        let copy = actions
            .iter()
            .find(|a| a["name"] == "copy")
            .expect("copy action");
        assert_eq!(copy["current"], json!("ctrl+y"), "the override is surfaced");
        let toggle = actions
            .iter()
            .find(|a| a["name"] == "toggle-sidebar")
            .expect("toggle-sidebar action");
        assert_eq!(toggle["current"], Value::Null, "an unbound action is null");
        assert!(
            toggle["default"].as_array().is_some_and(|d| !d.is_empty()),
            "defaults come through from core"
        );
    }

    #[test]
    fn keys_resource_is_served_over_resources_read() {
        let req = json!({
            "jsonrpc": "2.0", "id": 10, "method": "resources/read",
            "params": { "uri": KEYS_URI }
        });
        let resp = respond(&req, &json!({}));
        let text = resp["result"]["contents"][0]["text"]
            .as_str()
            .expect("keys text");
        let keys: Value = serde_json::from_str(text).expect("keys payload is JSON");
        assert!(keys["actions"].is_array());
    }

    #[test]
    fn resources_list_advertises_both_the_schema_and_keys() {
        let req = json!({ "jsonrpc": "2.0", "id": 11, "method": "resources/list" });
        let resp = respond(&req, &json!({}));
        let uris: Vec<&str> = resp["result"]["resources"]
            .as_array()
            .expect("resources array")
            .iter()
            .filter_map(|r| r["uri"].as_str())
            .collect();
        assert!(uris.contains(&SCHEMA_URI));
        assert!(uris.contains(&KEYS_URI));
    }
}
