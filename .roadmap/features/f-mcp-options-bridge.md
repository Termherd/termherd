+++
id = "F-mcp-options-bridge"
type = "feature"
area = ["mcp"]
status = "done"
target = ["Could"]
+++

`list_options` and `set_option` on the live bridge, not only the stdio slice.

Shipped as #298. A session termherd launched — the only kind it launches —
saw seventeen tools, none about settings, so "switch me to a light theme"
needed the stdio server registered by hand. The bridge now carries both over
the same pure catalogue in `crates/mcp` (one implementation, two transports;
the tool descriptions are pinned equal by a test, since `#[tool]` takes only
a literal). Neither surface writes over a `settings.json` that does not
parse. Paired with the settings hot reload (#297): this makes the change
reachable from a session, that one makes it visible without a restart. Rung
of [F-mcp-control-surface](#f-mcp-control-surface), after
[F-mcp-config-write](#f-mcp-config-write).
