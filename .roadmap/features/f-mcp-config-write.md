+++
id = "F-mcp-config-write"
type = "feature"
area = ["mcp"]
status = "done"
target = ["Could"]
+++

`set_option` and `keys` on the stateless stdio slice.

Shipped as #191. Config is a file, so this rung needed no live bridge — it was
independent of the rest of [F-mcp-control-surface](#f-mcp-control-surface) and
deliverable on its own.

Narrowed in #283: `shell.program` and `shell.args` are read-only over MCP —
their value is what the next launch executes — and `list_options` and the
schema resource carry a `writable` flag per id so a model can tell before it
tries. A value that does not fit an option's kind is refused with a JSON-RPC
error and nothing is written; it used to be written anyway with a warning.
