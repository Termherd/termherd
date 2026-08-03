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
selection (#159). Cmd/Ctrl-clickable **file paths** shipped on that same seam
(#252): `core::paths` finds a candidate syntactically, and a `PathResolver`
port checks it against the filesystem — the check, not the regex, is what keeps
prose like `and/or` from lighting up half the screen. Resolution tries the live
`cwd` from [F-terminal-cwd](#f-terminal-cwd), then the repository holding it,
then the launch directory, because `cargo`, `git` and `pytest` each print
relative to a different root. It opens through the OS handler, which leaves two
debts closing together in #257: the `:line` it parses is carried to the effect
but cannot be honoured, and opening by association means a path that *is* a
program must be refused rather than run — a refusal that is airtight on
macOS/Linux and mostly nominal on Windows, where the association table maps
every installed interpreter. Two contract bugs sit on the same surface: mouse reporting
isn't forwarded to the child (#155, vim) and the `emitted_lines_never_drift`
property has a known failing scroll sequence whose seed was never committed
(#102)
