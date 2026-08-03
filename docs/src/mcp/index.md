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
| Available to | sessions launched from TermHerd | any Claude session |
| Transport | in-process, loopback, per-session bearer token | JSON-RPC over stdio |
| Surface | 13 tools | 2 tools + 2 resources |
| Needs the app running | ✅ yes | ❌ no |

## The gap between them

Neither surface lets an agent running **outside** termherd drive the **running**
workspace. The live bridge holds the sessions, but only a session it spawned
holds the endpoint and token to reach it; the stdio server is reachable from
anywhere and sees only the settings file.

So the terminal that launched termherd cannot ask it for a screenshot the app
already knows how to take. Closing that is
[#267](https://github.com/Termherd/termherd/issues/267) — discovery, not new
tools.

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
