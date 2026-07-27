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
