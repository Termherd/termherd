+++
id = "F-notification-focus-tab"
type = "feature"
area = ["workspace", "sessions"]
status = "todo"
target = ["Should"]
+++

Clicking a tab's desktop notification brings termherd forward on that tab.

The click reveals the pane by its `SessionId`, through the path the MCP
`focus_pane` tool already takes, then raises the window (#352). No new
dependency: `notify-rust` already answers a click on all three OSes. Each OS
still needs a real click to confirm the window comes forward, and Windows
attributes the toast to PowerShell until termherd registers an application id.
Builds on [F-status-notifications](#f-status-notifications). Torture report:
`.personal/feature-torture/reports/F-notification-focus-tab.md`.
