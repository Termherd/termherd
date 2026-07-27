+++
id = "F-mcp-live-bridge"
type = "feature"
area = ["mcp"]
status = "done"
target = ["Could"]
+++

The gate: an in-process MCP server on loopback, reaching the live `core::App`.

**The gate, green.** An in-process MCP server on loopback
(`127.0.0.1:<ephemeral>/mcp`), reachable by the Claude sessions termherd
launches, answering tool calls over the #192 bridge into the live `core::App` —
core never touched directly. The `list_sessions` spike returns every live
session with a **stable external handle** (the runtime `SessionId`, minted once
and never re-keyed — decoupled from the Claude resume id that a
fork/plan-accept re-keys, Q6). A **per-session bearer token** (v4 UUID, OS
CSPRNG) gates every request, minted on each Claude launch and injected via the
session's `mcpServers` config (a `0o600` file, never argv, never logged) —
`Launch::Claude` carries the endpoint as opaque data on the spawn request; the
pty writes the config and passes `--mcp-config`. Proven end-to-end headless
(401 without a token, full `initialize` handshake with one, `list_sessions`
through the bridge) and by a real app run (server binds + auth-gates on
loopback). **Server crate = `rmcp`** (the official MCP Rust SDK — carries
JSON-RPC / handshake / tool routing so we never hand-roll the protocol),
Apache-2.0 (cargo-deny allows it), edition 2024, tokio-native (reuses the #192
runtime, now `enable_all` for the listener); the http stack is
`hyper`/`hyper-util` (frugal base — no axum). Unblocks #194/#195/#196
