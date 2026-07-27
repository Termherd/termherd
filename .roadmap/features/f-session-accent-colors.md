+++
id = "F-session-accent-colors"
type = "feature"
area = ["workspace", "sidebar"]
status = "todo"
target = ["Could"]
+++

A per-session accent on its tab, sidebar row and pane border.

Per-session / per-agent visual accents: give each session (or agent kind —
Claude, plain shell, `agy`) a colour used on its tab chip, sidebar row and pane
border, so parallel sessions are distinguishable at a glance. Chrome accents,
not grid colours — sibling of, but separate from, `F-terminal-palette`. Natural
home for the assignment is the `~/.termherd/metadata.json` overlay (like
`F-session-metadata`). **Design-first**
