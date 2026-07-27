+++
id = "F-builtin-terminal"
type = "feature"
area = ["terminal"]
status = "done"
target = ["Must"]
+++

A native PTY terminal per session, rendered on an iced canvas.

PTY + native terminal widget (M2): `termherd-pty` adapter (`portable-pty` +
`alacritty_terminal`, reader + terminal thread per session, cursor-report reply
for ConPTY); iced `canvas` renders the colour grid + cursor; raw keyboard
routed to the focused PTY; wheel scrollback; drag-to-select + copy; `claude
--resume` on a session click; PTY resize follows the window. Verified
end-to-end on Windows resuming a real Claude session. The widget shipped; the
**terminal ergonomics a user compares against Ghostty/iTerm are still open**,
each a refinement of this entry rather than a feature of its own: clipboard
conventions (copy-on-select, paste-on-right-click, #36), Cmd/Ctrl-clickable
hyperlinks whose displayed text differs from the URL (OSC 8, #84), auto-scroll
when a drag-selection reaches the canvas edge (#157), and Alt+drag rectangular
selection (#159). Two contract bugs sit on the same surface: mouse reporting
isn't forwarded to the child (#155, vim) and the `emitted_lines_never_drift`
property has a known failing scroll sequence whose seed was never committed
(#102)
