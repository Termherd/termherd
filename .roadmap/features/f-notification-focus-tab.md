+++
id = "F-notification-focus-tab"
type = "feature"
area = ["workspace", "sessions"]
status = "done"
target = ["Should"]
+++

Clicking a tab's desktop notification brings termherd forward on that tab.

Built in #352. **Confirmed by a real click on macOS** (2026-10-08): a Claude
session that finished in a background tab raised a notification, and clicking it
brought that tab forward. The minimised-window and open-prompt paths, and Linux
and Windows, are still read from the sources only. `Effect::Notify` carries the
`SessionId`; the `os-notify` thread that posts the notification waits for the
OS's answer, and a body click sends the session to the shell over a channel an
iced subscription drains. The shell reveals the pane through the path the MCP
`focus_pane` tool takes — unless a prompt is open, which keeps its screen — then
restores and raises the window. A session closed in the meantime reveals nothing
but still raises the window.

Per OS, from the sources: macOS waits through
`mac-notification-sys` directly, since notify-rust 4.18's `wait_for_response`
there returns "expired" at once and never sees the click; XDG needs the
`"default"` action declared and replaces a session's notification in place,
so one waiter per session; Windows answers `Expired` when a toast times out
into the action centre, so a click from there is lost, and the toast is
attributed to PowerShell until termherd registers an application id. A waiting
thread cannot be cancelled from outside the backend, so at most 16 wait at
once; past that a notification posts without a click. Builds on
[F-status-notifications](#f-status-notifications). Torture report:
`.personal/feature-torture/reports/F-notification-focus-tab.md`.
