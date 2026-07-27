+++
id = "F-mcp-snapshot-g1"
type = "feature"
area = ["mcp", "workspace"]
status = "done"
target = ["Could"]
+++

One model, two readers: the capture dump is now the MCP snapshot.

**One model, two readers.** The G1 dev-loop dump
(`~/.termherd/captures/capture-<ts>.json`) is now the same `WorkspaceSnapshot`
the MCP `snapshot` tool reports, under a fixed `SnapshotFilter::capture()`
(every section, the focused pane's screen kept whole — a file pays for the full
picture where a call does not). `CaptureDump`/`CaptureTab` and `core::capture`
are gone: `Event::Capture` carries the adapter-injected `SnapshotInputs`,
`Effect::Capture` the snapshot, and one JSON wire form (`app::snapshot_dto`,
extracted from the MCP handler) serves both readers — so the dump gained
config, the sidebar and per-pane detail, and a field can no longer mean one
thing on disk and another on the wire. Breaking dump-format change
(`focused_pty` → `terminals` by handle; status words follow the MCP
vocabulary, `ready` → `idle`); the newest-stamp discovery contract is
untouched. Depends on #212
