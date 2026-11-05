+++
id = "F-url-scheme"
type = "feature"
area = ["workspace", "packaging"]
status = "todo"
target = ["Could"]
+++

Open a tab from an HTML link: a `termherd://` URL scheme.

Filed as #446. A page (docs, a dashboard, a report) links straight into the
running instance. Three pieces: the scheme registered with each OS, a relay
from a second launch to the running instance, and a target key that survives a
restart (the MCP `handle` does not). The terminal already detects the scheme
as a link. Discovery from outside overlaps [F-mcp-attach](#f-mcp-attach).
