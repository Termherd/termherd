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
each a refinement of this entry rather than a feature of its own: auto-scroll
when a drag-selection reaches the canvas edge (#157), and Alt+drag rectangular
selection (#159). **Hidden hyperlinks** (OSC 8, #84) closed on that list: the
`Screen` snapshot now carries every hyperlink as a span over the cells it
covers, read off alacritty's own parse rather than re-detected, and the pointer
answers that span before it scans the row's text — so a label like `#76`
underlines and opens its issue URL exactly as a printed URL does. The span
rides beside the grid rather than in each cell, which keeps `ScreenCell` a
plain `Copy` value and mirrors how the selection already travels. The
**clipboard mouse conventions** closed on that list
(#36): `terminal.copy_on_select` and `terminal.paste_on_right_click`, both off
by default. Copy-on-select was in fact already shipped and unconditional, so
the setting *added the off position* — the automatic copy is now opt-in and the
copy chord is the default path. The paste is addressed to the pane under the
pointer rather than the focused one, which is why it needed a session-carrying
message rather than a second call into the existing paste; both paths now meet
at one `paste_into` seam that decides bracketing once. Gating the automatic
copy also disabled the copy *chord*, which read only a cache the automatic copy
filled — so the chord now reads the terminal's live selection and prefers it,
which fixes the older bug of a stale cache outranking a fresh highlight too.
**Two earlier records read as contradictions and are not**: the M2 line above
says "drag-to-select + copy", and `CHANGELOG.md` says "copied on release" under
a released `0.1.0-prerelease` heading. Both are accurate as dated records of
what shipped then; the default changed afterwards, and the `[Unreleased]`
section carries the correction. Cmd/Ctrl-clickable
**file paths** shipped on that same seam
(#252): `core::paths` finds a candidate syntactically, and a `PathResolver`
port checks it against the filesystem — the check, not the regex, is what keeps
prose like `and/or` from lighting up half the screen. Resolution tries the live
`cwd` from [F-terminal-cwd](#f-terminal-cwd), then the repository holding it,
then the launch directory, because `cargo`, `git` and `pytest` each print
relative to a different root. It opened through the OS handler, which left two
debts that closed together in #257 — an `open.command` in `settings.json`, with
`{path}` / `{line}` / `{col}` templates. Configured, it replaces the handoff:
the `:line` the terminal printed is finally honoured, and since an explicit
command consults no file association, the refusal that kept a path which *is* a
program from being opened has nothing left to protect and is lifted with it.
That refusal was airtight on macOS/Linux and mostly nominal on Windows, where
the association table maps every installed interpreter — so the command does not
narrow that gap, it removes it. The command is argv and never a shell line: the
configured string is split on whitespace *before* `{path}` is substituted, so a
filename cannot become a second argument, and a placeholder in the program name
is refused outright — what the terminal printed picks the file, never the
executable. Unconfigured, the OS handoff and its refusal both stand.
Mouse buttons reach the child since #155: a program with mouse reporting on
gets presses, releases, drags and motion in the SGR or X10 encoding it
negotiated, the terminal selects nothing of its own over it, and Shift takes
the mouse back for a local selection. The copy chord used to fall back to a
cache of the last text copied when no selection was visible, which in a pane
the child owns — never a visible selection — overwrote the child's own copy on
every chord (#316); the cache is gone and the chord asks the terminal whether
it holds a selection, scrolled out of view or not. One contract bug remains on
the surface: the `emitted_lines_never_drift` property has a known failing
scroll sequence whose seed was never committed (#102)
