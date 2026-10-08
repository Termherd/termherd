+++
id = "F-session-id-at-launch"
type = "feature"
area = ["sessions"]
status = "done"
target = ["Should"]
+++

A fresh Claude tab knows its session id from the first keystroke.

Before, a fresh tab carried no Claude id, so every feature that reads a
session's JSONL — [F-tab-title-sync](#f-tab-title-sync),
[F-session-accent-colors](#f-session-accent-colors),
[F-tab-hover-details](#f-tab-hover-details),
[F-prompt-history](#f-prompt-history), [F-session-reveal](#f-session-reveal) —
did nothing there (#336).

**Shipped in #336, both sources, layered.** The shell mints a v4 UUID for every
fresh launch and the launch line gains `claude --session-id <uuid>` (validated
at the argv seam: a non-UUID is dropped, never typed). The per-pane session
file `~/.claude/sessions/<pid>.json`, the reader
[F-copy-agent-name](#f-copy-agent-name) built, outranks the minted id whenever
it proves the Claude in front, since Claude rewrites it on a re-key. One
accessor, `LiveSession::claude_session_id` (with `App::claude_session_id` and
`App::tab_claude_session_id`), is what every reader of a pane's transcript
goes through; the last id a file proved outlives the Claude that wrote it.
Reopening a closed Claude tab resumes its last conversation when the scan has
it, else starts a new one under a new id. The CLI floor rose to 2.0.73, an
estimate: the oldest release whose changelog shows `--session-id` in use.

Not verified: whether `/clear` or a plan-accept re-keys a session started with
`--session-id`. If it does, the session file is what follows it.
