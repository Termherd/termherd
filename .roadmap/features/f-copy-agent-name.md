+++
id = "F-copy-agent-name"
type = "feature"
area = ["sessions", "workspace"]
status = "todo"
target = ["Could"]
+++

Copy a session's agent name, the one `/list-agents` shows.

The peer name Claude Code gives a session (`termherd-b0`) is how one session
addresses another (#339). Read from `~/.claude/sessions/<pid>.json`, through
the pid-to-session-file reader #333 needs anyway. Also shown in
[F-tab-hover-details](#f-tab-hover-details).
