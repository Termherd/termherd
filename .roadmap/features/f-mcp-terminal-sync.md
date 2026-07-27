+++
id = "F-mcp-terminal-sync"
type = "feature"
area = ["mcp", "terminal"]
status = "done"
target = ["Could"]
+++

The wait rung: block until a session's status settles, then read its text.

**The wait rung**, closing act→**wait**→observe. `wait_for_status` blocks until
a session's OSC-derived activity reaches one of the asked-for statuses (default
idle-or-attention), and `read_terminal` returns one pane's visible text — the
deep read the light `snapshot` leaves out. The wait is the first bridge request
whose reply lands in a *later* `update`: the shell parks the reply port in a
waiter list and settles it from the status change, so nothing is polled and no
transition is raced (the feature-torture report cut `wait_for_text` for exactly
that). Three rules earn their tests: a session already on the target answers at
once (never wait for a transition already past); an exit settles every waiter
whatever was asked for (a crash emits no status change, and a dead session will
not reach the target); a caller that gave up is swept off the list. Timing out
is not an error — the tool reports `{ status, timed_out: true }` with the
session's current status, so an agent can choose between waiting again and
giving up. Bounds are the caller's: `timeout_ms` (default 30 s) capped at 5 min
(Q7). Depends on #193; unblocks #196
