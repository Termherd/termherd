+++
id = "F-session-id-at-launch"
type = "feature"
area = ["sessions"]
status = "todo"
target = ["Should"]
+++

A fresh Claude tab knows its session id from the first keystroke.

Today a fresh tab carries no Claude id, so every feature that reads a
session's JSONL — [F-tab-title-sync](#f-tab-title-sync),
[F-session-accent-colors](#f-session-accent-colors),
[F-tab-hover-details](#f-tab-hover-details),
[F-prompt-history](#f-prompt-history), [F-session-reveal](#f-session-reveal) —
does nothing there (#336). Two sources: launch with
`claude --session-id <uuid>`, or read the `sessionId` Claude Code writes to
`~/.claude/sessions/<pid>.json`, through the reader #333 needs anyway. The
session file also carries the agent name `/list-agents` shows, which favours
it; to settle before building.
