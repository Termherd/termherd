# Attaching from outside

The [live bridge](./live-bridge.md) is wired into a Claude session at spawn:
TermHerd mints a token, writes it into that session's `mcpServers` config, and
hands the file to the child it is launching. Nothing about the server refuses
another caller — it accepts any token it minted, whoever presents it. What a
caller outside that process tree lacks is the **address**: an ephemeral port it
cannot learn, and a token it was never handed.

Attaching publishes that address, once, for the whole instance. The tools are
unchanged — the same thirteen, reached from somewhere else.

## Turning it on

Off by default. In `~/.termherd/settings.json`:

```json
{ "mcp": { "attach": true } }
```

Restart TermHerd. It then writes, and keeps current for its lifetime:

```json
// ~/.termherd/bridge.json — mode 0600
{ "url": "http://127.0.0.1:52431/mcp", "token": "9f1c…" }
```

One instance runs at a time, so the file needs no "which instance" field. It is
removed when the app exits.

For a one-off run — an automation script, a CI-ish sweep — the environment
variable flips the setting without editing it:

```bash
TERMHERD_MCP_ATTACH=1 termherd    # publish for this launch only
TERMHERD_MCP_ATTACH=0 termherd    # and back off, with attach: true on file
```

`1` / `true` / `yes` / `on` and `0` / `false` / `no` / `off` are all understood,
case-blind. Anything else is not an override at all: the setting stands.

## Refusing it outright

```json
{ "mcp": { "attach": "never" } }
```

`"never"` is not a third spelling of `false`. It is off **and** out of reach of
the environment variable, so a machine — a shared box, a build agent, a laptop
under someone else's policy — can decline the exposure in a way an ambient
`TERMHERD_MCP_ATTACH=1` cannot quietly undo. An override attempted against it is
ignored and logged at `info`, so nobody is left wondering why the variable did
nothing.

## Registering a client

Do not read `bridge.json` by hand. Ask the running instance:

```console
$ termherd mcp-config
{"mcpServers":{"termherd":{"type":"http","url":"http://127.0.0.1:52431/mcp","headers":{"Authorization":"Bearer 9f1c…"}}}}
```

That is the `mcpServers` snippet, byte-for-byte the shape a spawned session
receives — one contract, two producers. Pipe it where your client wants it:

```bash
termherd mcp-config > .mcp.json          # a project-local MCP config
claude --mcp-config <(termherd mcp-config)
```

The subcommand prints to stdout and exits `0`; it answers *about* the running
instance without disturbing it — no window opens and the single-instance lock is
never touched.

### When there is nothing to print

The command **probes** the endpoint with a real MCP `initialize` before printing
anything. A file left behind by a crash names a port nothing answers on, and a
config built from it would fail on the first call instead of the zeroth. Four
failures, four different things to do:

| stderr says | What happened | Do |
| --- | --- | --- |
| no running termherd is publishing… | no file: not running, or not opted in | set `mcp.attach` and restart |
| …could not be read | the file exists and does not parse | restart to rewrite it |
| …no longer answering | the instance that wrote it has exited | restart |
| …refused its own token | the endpoint is live but rejects the token | restart to publish a fresh one |

Each exits non-zero, so a script can tell "no config" from a config.

## What you are exposing

Worth stating plainly, because the default is `false` for this reason and no
other.

The published file hands any process running as you the ability to drive your
workspace: `run_in_session` types into a terminal, which is arbitrary command
execution under your account, and `read_terminal` reads back whatever is on
screen. The file is `0600`, which keeps *other* users out; it does not keep out
anything running as you.

This is not a new capability — a process that can read a spawned session's
`--mcp-config` file already has the same reach. Attaching makes it
*discoverable* rather than newly possible. That distinction is a reason to say
so out loud, not a reason to skip the gate: on a machine where that trade is
wrong, `"never"` is the answer.
