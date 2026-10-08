+++
id = "F-notification-focus-tab"
type = "feature"
area = ["workspace", "sessions"]
status = "done"
target = ["Should"]
+++

Clicking a tab's desktop notification brings termherd forward on that tab.

Shipped in #352. `Effect::Notify` carries the `SessionId`; the `os-notify`
thread that posts the notification waits for the OS's answer, and a body click
sends the session to the shell over a channel an iced subscription drains. The
shell reveals the pane through the path the MCP `focus_pane` tool takes, then
raises the window with `window::gain_focus`; a session closed in the meantime
reveals nothing but still raises the window. No new dependency: `notify-rust`
answers a click on all three OSes, and only XDG needs the `"default"` action
declared. A notification nobody answers keeps its thread parked (macOS keeps it
in the notification centre, a Windows toast in the action centre may never
answer), and the thread cannot be cancelled from outside the backend, so at
most 16 notifications wait for a click at once; past that they post without
one. Each OS still needs a real click to confirm the window comes forward, and
Windows attributes the toast to PowerShell until termherd registers an
application id. Builds on [F-status-notifications](#f-status-notifications).
Torture report: `.personal/feature-torture/reports/F-notification-focus-tab.md`.
