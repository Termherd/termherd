+++
id = "F-mcp-attach"
type = "feature"
area = ["mcp", "workspace"]
status = "todo"
target = ["Could"]
+++

The attach rung: reach the live bridge from outside a spawned session.

Filed as #267. **The rung the ladder never had.** Every other rung of
[F-mcp-control-surface](#f-mcp-control-surface) assumes the caller is a Claude
session **termherd launched** — that is how it holds a token and an endpoint at
all, both injected into its `mcpServers` at spawn. An agent running anywhere
else, including the very terminal that launched termherd, sees nothing: an
ephemeral port it cannot learn and a per-session token it was never handed.

The asymmetry is invisible until you meet it, and then it is total. It is what
made the documentation screenshots (#264) unautomatable: the app can screenshot
itself and press its own keys, and the one process that needed to ask it to —
the launcher — was the one process with no way to speak to it. `press_keys`,
`screenshot` and `wait_for_status` were all there, all unreachable.

So the missing piece is **discovery, not capability**. No new tool: the same
thirteen, reached by a client that was not spawned as a child.

**The shape, to settle.** The pieces are small and their arrangement is not:

- **Where a running instance publishes itself.** A `0o600` file beside the rest
  of termherd's state (`~/.termherd/`) carrying `{ url, token }`, written at
  startup and removed at teardown, is the obvious answer — and inherits the
  single-instance lock, which is why "which instance?" needs no field. A stale
  file after a crash must read as stale rather than as an endpoint.
- **How a client registers it.** A subcommand emitting the `mcpServers` snippet
  (`termherd mcp-config`) keeps the token out of shell history and out of argv,
  where an `--print-token` flag would put it in both.
- **Whether the surface is the same thirteen.** Probably yes: a narrower
  read-only surface would be a second contract to keep true, and the
  interesting uses (drive, wait, screenshot) are the mutating ones.

**The security delta is the real design question, and it is not zero.** A
spawned session's token reaches it through a file only that session's process
tree is handed; a published endpoint is discoverable by anything running as the
user. The capability behind it is not modest — `run_in_session` types into a
terminal, which is arbitrary command execution as the user, and `read_terminal`
reads back whatever is on screen. That is already reachable by any process that
can read a session's `--mcp-config` file, so the change is *discoverable by
default* rather than *newly possible* — but "already true elsewhere" is an
argument for stating the exposure, not for skipping it. Expect an opt-in
setting rather than an always-on file, and expect that decision to be the one
this feature actually turns on.

Adjacent and **not** the same thing:
[F-mcp-ide-bridge](#f-mcp-ide-bridge) runs the other way round (termherd as
the *client* of Claude's IDE bridge), and the stdio `termherd-mcp` server
already reaches any session from anywhere — but only the settings file, never
the running workspace, which is exactly the gap this closes.
