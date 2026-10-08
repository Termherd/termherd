+++
id = "F-keymap-rename-tab"
type = "feature"
area = ["keymap", "workspace"]
status = "done"
target = ["Could"]
+++

A `rename-tab` keymap action opening the focused tab's inline rename.

Rename was double-click only, so neither the keyboard, the
[F-tab-context-menu](#f-tab-context-menu) nor MCP `run_action` could reach it
(#338).

Shipped (#338): `rename-tab`, bound to ⌘⇧I on macOS (Terminal.app's *Edit
Title*) and Ctrl+Shift+I elsewhere, a chord a legacy-encoded terminal program
cannot tell from Ctrl+I (Tab). It opens the field a double-click opens, filled
with the tab's current name, and reports `inert` / `no-context` with no tab
open. Over MCP the field can be opened and abandoned with `escape`, not
committed: `enter` on a rename is #246.
