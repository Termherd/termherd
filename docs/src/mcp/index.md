# Two surfaces

TermHerd exposes itself to Claude sessions over
[MCP](https://modelcontextprotocol.io), so a session can read and drive the
workspace it is running in. There are **two servers**, and which one you get
depends on how the session started.

```text
          ┌──────────────────────────────┐
          │        TermHerd (GUI)        │
          │  ┌────────────────────────┐  │
 in-proc  │  │   core::App (state)    │  │
 loopback │  └───────────▲────────────┘  │
   ┌──────┼──────────────┘               │
   │      │   the LIVE BRIDGE            │
   │      │   13 tools · the running     │
   │      │   workspace                  │
   │      └──────────────────────────────┘
   │
   │  wired in at spawn, per-session token
   ▼
┌─────────────────────┐        ┌──────────────────────────┐
│ a Claude session    │        │  any Claude session      │
│ LAUNCHED BY termherd│        │  anywhere                │
└─────────────────────┘        └────────────┬─────────────┘
                                            │ you register it
                                            ▼
                               ┌──────────────────────────┐
                               │  termherd-mcp (stdio)    │
                               │  2 tools · settings.json │
                               └──────────────────────────┘
```

| | [The live bridge](./live-bridge.md) | [The stdio server](./stdio.md) |
| --- | --- | --- |
| Reaches | the **running** workspace | the **settings file** |
| Setup | none — wired in at spawn | you register `termherd-mcp` yourself |
| Available to | sessions launched from TermHerd, plus any client that [attaches](./attach.md) | any Claude session |
| Transport | in-process, loopback, per-session bearer token | JSON-RPC over stdio |
| Surface | 13 tools | 2 tools + 2 resources |
| Needs the app running | ✅ yes | ❌ no |

## The gap between them, and the bridge across it

Two servers, but three ways in. The live bridge holds the running workspace and
hands its endpoint only to sessions it spawned; the stdio server is reachable
from anywhere and sees only the settings file. Between those sat a hole: the
terminal that launched TermHerd could not ask it for a screenshot the app
already knows how to take.

[Attaching from outside](./attach.md) closes it — the **same** live bridge and
the same thirteen tools, reached by a client TermHerd did not spawn. What was
missing was discovery, not capability, so nothing new was added to the surface.
It is opt-in (`mcp.attach`), because publishing an endpoint that can type into
a terminal is a decision, not a default.

## Which one you want

- *"Split this pane and run the tests"* → **live bridge**. It only exists
  inside a session TermHerd started.
- *"Switch me to a light theme"* → **stdio server**. It edits the file; the
  change applies on restart.

## Design notes

**Sessions are addressed by a stable `handle`** — the runtime session id —
never by the Claude `resume_id`, which re-keys on a fork or a plan-accept.

**Every call is timeout-bounded.** A wedged shell surfaces as a tool error,
never a hang. Waits have a default *and* a hard cap, so no call, however
parameterised, can park a caller indefinitely.

**The domain core has no MCP awareness at all.** Every mutation goes through an
`Event` that already existed for the keyboard. That is the invariant behind the
whole surface: an MCP caller cannot reach a state the keyboard cannot.
