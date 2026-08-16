+++
id = "F-mcp-attach"
type = "feature"
area = ["mcp", "workspace"]
status = "done"
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

**The shape, as shipped.** The pieces are small and their arrangement was not:

- **Where a running instance publishes itself.** A `0o600`
  `~/.termherd/bridge.json` carrying `{ url, token }`, written after the
  single-instance lock and removed at teardown — so "which instance?" needs no
  field. A crash skips the teardown, which is why the reader probes.
- **How a client registers it.** `termherd mcp-config` prints the `mcpServers`
  snippet on stdout, keeping the token out of argv and out of shell history
  where an `--print-token` flag would have put it in both. It is recognised
  before the single-instance lock is taken and before any window opens: a
  question about the running instance must not disturb it.
- **How a stale file reads as stale.** By an MCP `initialize` against the
  published endpoint, not by a pid. A cheaper proxy can pass while the endpoint
  refuses every real call; probing with the operation actually intended cannot.
  A refused token (401) is reported apart from a dead port, since the two send
  the caller to opposite places.
- **The surface is the same thirteen.** A narrower read-only variant would be a
  second contract to keep true, and the interesting uses (drive, wait,
  screenshot) are the mutating ones.

**The security delta was the real design question, and it is not zero.** A
spawned session's token reaches it through a file only that session's process
tree is handed; a published endpoint is discoverable by anything running as the
user. The capability behind it is not modest — `run_in_session` types into a
terminal, which is arbitrary command execution as the user, and `read_terminal`
reads back whatever is on screen. That is already reachable by any process that
can read a session's `--mcp-config` file, so the change is *discoverable by
default* rather than *newly possible* — but "already true elsewhere" is an
argument for stating the exposure, not for skipping it.

So publishing is **opt-in and tri-state**: `mcp.attach` takes `false` (the
default), `true`, or `"never"`. `TERMHERD_MCP_ATTACH` flips the first two for a
single launch, which is what an automation run wants; against `"never"` it is
inert and says so at `info`, which is what a shared machine wants — a policy an
ambient environment variable cannot quietly undo. That third state is the whole
reason the setting is not a boolean, and it stays file-only: a surface able to
widen its own reach is not a setting.

Adjacent and **not** the same thing:
[F-mcp-ide-bridge](#f-mcp-ide-bridge) runs the other way round (termherd as
the *client* of Claude's IDE bridge), and the stdio `termherd-mcp` server
already reaches any session from anywhere — but only the settings file, never
the running workspace, which is exactly the gap this closes.
