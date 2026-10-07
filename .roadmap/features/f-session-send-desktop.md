+++
id = "F-session-send-desktop"
type = "feature"
area = ["sessions"]
status = "todo"
target = ["Could"]
+++

Continue a session in Claude Desktop by sending it `/desktop`.

Sent through [F-claude-command](#f-claude-command), from a keymap action and
the [F-tab-context-menu](#f-tab-context-menu) (#347). The tab stays open on
its shell afterwards, since success is not observable. macOS and Windows x64
only. Torture report:
`.personal/feature-torture/reports/F-session-send-desktop.md`.
