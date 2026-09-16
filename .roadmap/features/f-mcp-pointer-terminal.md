+++
id = "F-mcp-pointer-terminal"
type = "feature"
area = ["mcp", "terminal"]
status = "done"
target = ["Could"]
+++

The pointer rung, terminal half: place a mouse event inside a session.

**The pointer rung, terminal half.** Place a mouse event **inside a session's
terminal**, the way `run_in_session` places text there. Shipped as
`mouse_in_session(session, kind, col, row, button?)` (#300). Cell
addressed — a terminal is a grid, and a grid is what an SGR report carries — and
bounded by the pane's last rendered geometry: a cell outside it rejects the
whole call before anything applies, as a malformed chord does for `press_keys`.
The answer says what the terminal did with it: `selection` when it drove the
local text selection, `ignored` when the event maps to no local gesture (a
release, a move, a button other than the left one).

Two gaps motivated it. The act→wait→observe loop had no pointer at all, so
every mouse-mode TUI a session hosts — Claude Code's `/diff` and `/resume`,
lazygit, fzf, vim — was unreachable to an agent whose keyboard already worked.
And #155 (mouse clicks are never encoded to the child) could not be verified
end to end by the agent that fixes it: its encoder half unit-tests the way the
wheel's already does in `pty::input`, but the gesture itself against a real
child is what decides whether a TUI actually responds. **Blocks #155**, which
is why it landed first.

It shares one seam with that bug, and the rung built the seam without the
second copy the design feared. The path mirrors the wheel's end to end —
`Event::TerminalPointer` → `Effect::TerminalPointer` → `PtyHost::pointer` → the
per-session terminal thread, which holds the live scroll offset — and the
gesture rule lives once in `core::app::pointer`: *whether* an event drives the
selection is read off the event (that is what the shell answers), *where* it
lands is placed by the terminal with its live offset. #155 then *extended*
that arm as planned: `PointerEvent::route` reads the child's mouse reporting
and answers forward / select / nothing, the SGR/X10 encoder grew from
`wheel_bytes` into `mouse_bytes`, `forwarded` is the third answer, and the
canvas hands its own bare press / drag / release / move down the same path
while Shift keeps a local selection possible.

Sibling of [F-mcp-pointer-chrome](#f-mcp-pointer-chrome), which drives
termherd's own interface rather than a terminal and blocks nothing.
