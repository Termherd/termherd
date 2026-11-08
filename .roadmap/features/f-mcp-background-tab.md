+++
id = "F-mcp-background-tab"
type = "feature"
area = ["mcp", "workspace"]
status = "done"
target = ["Could"]
+++

Open and close a tab over MCP without moving the user's focus.

**Background open and close** (#363). Every action of
[F-mcp-orchestration](#f-mcp-orchestration) moved the keyboard: an open
activated its new tab, a close focused its target first. An agent running
workers beside a human sent that human's next keys into a terminal they did
not choose. One optional `background` flag (default `false`) on `open_session`
and `close_pane` fixes it without a new tool and without changing the default.

A background open appends its tab without activating it — a `Placement` on
`core`'s `LaunchSpec` — and sizes its PTY to the tab area at once, so a Claude
started there never draws its first screen for the default grid. A background
close goes through a public `Event::ClosePane(SessionId)` over the existing
`Workspace::close_pane_of`, never revealing the pane, and requires `pane`: the
focused pane is the user's. The flag means *never take focus*, not *focus
cannot move* — closing the focused pane still hands focus to its sibling.
Every `open_session` reply now carries `opened_handle`, since `focused_handle`
no longer names the new session.

Answered probes: `mouse_in_session` reaches a never-drawn tab, since a
terminal's screen fills from its output rather than from drawing. An MCP close
asks no confirmation, background or not; the book says so rather than the
behaviour changing. An "opened by an agent" cue on the tab stays out of scope.
Follow-up candidate: the same flag on `split_pane`.
