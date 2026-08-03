+++
id = "F-mcp-control-surface"
type = "feature"
area = ["mcp"]
status = "todo"
target = ["Could"]
+++

Termherd exposes its own control and orchestration surface as an MCP server.

Termherd *exposes* an MCP server over its own control/config + orchestration
surface, driven by the in-app Claude sessions (termherd is the server, the
session is the client). Inverse of `F-mcp-ide-bridge`. Filed as #90 (now the
**tracking epic**). Tortured 🧬 **split** (feature-torture
`F-mcp-control-surface.md`; design brainstorm
`brainstorm/20260713-mcp-agent-terminal-interaction.md`): the entry hid
multiple features separated by the **transport** — config is *stateless* (a
file), orchestration/perception/synchro need the live `core::App` → an
in-process **http/sse** server (supersedes the earlier per-session-WS
assumption; Claude's MCP client speaks `stdio | http/sse`). A first,
**read-only** stdio slice has landed: `crates/mcp` (`termherd-mcp`),
`list_options` + schema resource, pure and unit-tested. Split into rungs, each
shippable:

- [x] [F-mcp-config-write](#f-mcp-config-write) — `set_option` and `keys` on
  the stateless stdio slice.
- [x] async transport substrate (#192, intrinsic quality) — tokio runtime in the
  composition root + a timeout-bounded request/reply primitive drained through
  the iced loop into `core::App` (pure state read → reply). The bound covers
  the enqueue too, so a full request channel can't hang the caller (Q7).
  Substrate-only: proven end-to-end by an in-process test transport, no live
  server yet. **Runtime = tokio** (MIT, ecosystem default; async-std is
  deprecated), feature-frugal (`rt-multi-thread`/`sync`/`time`/`macros`); the
  http/sse **server crate** pick (`tiny_http` vs `axum`/`hyper`) is deferred
  to #193, when the real transport lands and can be measured against
  MIT/no-FFI/frugal. Shared enabler, also unblocks `F-mcp-ide-bridge`;
  relates to #167/#171
- [x] [F-mcp-live-bridge](#f-mcp-live-bridge) — The gate: an in-process MCP
  server on loopback, reaching the live `core::App`.
- [x] [F-mcp-snapshot](#f-mcp-snapshot) — The perception rung: a filterable,
  light-by-default view of the whole app.
- [x] [F-mcp-orchestration](#f-mcp-orchestration) — The action rung: six
  mutating tools, each over an existing `core::App` event.
- [x] [F-mcp-terminal-sync](#f-mcp-terminal-sync) — The wait rung: block until
  a session's status settles, then read its text.
- [ ] [F-mcp-agent-loop](#f-mcp-agent-loop) — The composed prompt→wait→read
  over any session, shell or Claude.
- [x] [F-mcp-keys](#f-mcp-keys) — The keyboard rung: drive the app by key
  chords through the real keymap.
- [x] [F-mcp-screenshot](#f-mcp-screenshot) — The pixel rung: the window as a
  PNG, for what text cannot answer.
- [x] [F-mcp-snapshot-g1](#f-mcp-snapshot-g1) — One model, two readers: the
  capture dump is now the MCP snapshot.
- [ ] [F-mcp-attach](#f-mcp-attach) — The attach rung: reach the live bridge
  from outside, not only from a session it spawned.
