+++
id = "F-copy-agent-name"
type = "feature"
area = ["sessions", "workspace"]
status = "done"
target = ["Could"]
+++

Copy a session's agent name, the one `/list-agents` shows.

The peer name Claude Code gives a session (`termherd-b0`) is how one session
addresses another (#339). Read from `~/.claude/sessions/<pid>.json`, through
the pid-to-session-file reader #333 built. Also shown in
[F-tab-hover-details](#f-tab-hover-details).

Shipped (#339): a `copy-agent-name` action, unbound by default and reachable
through `run_action`, which re-reads the file on every press and is inert
(`no-context`) when no Claude in front has written one; and an `Agent:` line
in the tab hover card, read from a cache refreshed when the foreground job
changes and when the pointer enters the tab. The tab menu entry waits
for #340.

Not shipped: Windows. ConPTY reports no foreground process, so no pid leads to
a session file and nothing is ever named there; finding the Claude pid another
way is #357. Nor does a build launched from inside a Claude session name
anything, since its PTYs inherit that session's `CLAUDE*` environment (#356).
