+++
id = "F-terminal-cwd"
type = "feature"
area = ["terminal", "mcp", "sessions"]
status = "done"
target = ["Should"]
+++

The shell announces the directory it is in, so a session's `cwd` follows a `cd`.

**#251 — a field that was wrong by construction, not occasionally.**
`LiveSession.cwd` was written at launch and at split and never again, and
nothing in the tree decoded OSC 7 — so `PaneSnapshot.cwd`, which documents
itself as the path the session runs in, was false from the user's first `cd`
onward, with no signal that anything had moved. It has four readers and the
clicked-file feature that surfaced it is none of them: the MCP `snapshot` (so
any agent building a relative command), the ⌘⇧S capture written to disk, the
directory a split inherits, the "new shell / new Claude here" shortcuts, and
the tab card.
That is why it graduated as its own entry rather than as a slice of
[F-builtin-terminal](#f-builtin-terminal)'s file-link refinement — the lie
exists without the click, and closing it is worth doing without waiting for
one.

The fix rides the seam that already existed: the shell-integration recipe that
teaches zsh, bash and fish to emit OSC 133 now emits OSC 7 from the same prompt
hook, reading the shell's own live `$PWD` — a path baked into the generated
file would have reported the launch directory forever, which is the very bug.
Decoding sits in `pty` beside the prompt marks rather than in the `claude`
codec the ticket suggested: OSC 7 is the *shell's* dialect and the CLI never
writes it, so only the wire scan (`osc_sequences`) is shared, exactly the
sharing that scan's doc-comment already described. One field, replaced in
place, because all four readers mean *where the session is* — a second
`launch_cwd` beside it would have been a second thing to keep true with no one
asking for the first. A shell termherd has no recipe for (nu, pwsh) announces
nothing and keeps its launch directory: the same degradation its status
already takes, not a regression.

The generated line is POSIX shell producing a `file://` url, so its separators
are written by hand — the replay fix's lesson applied to a second string, where
a host separator would have broken both grammars at once. The decoder is
deliberately forgiving on the way in: it resolves percent escapes as *bytes*
(one `%C3%A9` is one character, not two), and keeps a `%` that starts no valid
escape as itself, since a legal filename character must not cost the whole
announcement and strand the session on a directory the user has left. The
end-to-end proof is a real PTY running the platform's own shell with a typed
`cd` — the only test that can tell whether the recipe reached the user's shell
at all; the unit tests below it assert a snippet or a decoded string, neither
of which knows if the hook ever ran.
