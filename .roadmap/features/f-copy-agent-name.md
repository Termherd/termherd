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
changes and when the pointer enters the tab. The tab menu entry shipped
with #340.

Windows shipped with #357. ConPTY reports no foreground process, so the Claude
pid is found by walking the process tree below the pane's shell (the nearest
`claude.exe`, else the shell's first child), and its start time is read as the
`FILETIME` Claude stamps into `procStart`. Not shipped: a Claude started there
through a wrapper script, which neither test finds. Nor does a build launched
from inside a Claude session name anything, since its PTYs inherit that
session's `CLAUDE*` environment (#356).
