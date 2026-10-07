+++
id = "F-session-accent-colors"
type = "feature"
area = ["workspace", "sidebar"]
status = "todo"
target = ["Could"]
+++

A per-session accent on its tab, sidebar row and pane border.

Per-session visual accents: a colour on a session's tab chip, sidebar row and
pane border, so parallel sessions are distinguishable at a glance. Chrome
accents, not grid colours — sibling of, but separate from,
`F-terminal-palette`. The kind is shown by
[F-tab-kind-icon](#f-tab-kind-icon), so colour stays free for the session.

Scoped into two slices. For a Claude tab the colour is the one Claude Code's
`/color` set — the last `agent-color` entry in the transcript — with no local
copy (#342). Picking a colour sends `/color` to a Claude tab through
[F-claude-command](#f-claude-command) and stores it on a shell tab, which
Claude knows nothing of (#343). Both use the same eight-colour palette as
`/color`. Torture report:
`.personal/feature-torture/reports/F-session-accent-colors.md`.
