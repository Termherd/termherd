# The stdio server

`termherd-mcp` is a separate, small binary that exposes TermHerd's
**configuration** — so you can ask "what can I configure here?", or "switch me
to a light theme", from *any* Claude session, whether or not TermHerd is
running.

It speaks JSON-RPC over stdio and is stateless: it reads and writes
`~/.termherd/settings.json`, nothing else.

## Registering it

```bash
cargo build -p termherd-mcp        # lands in target/
```

Add it to your `mcpServers` config, pointing `command` at the built binary:

```json
{
  "mcpServers": {
    "termherd": { "command": "/path/to/termherd-mcp" }
  }
}
```

## Tools

| Tool | Args | Does |
| --- | --- | --- |
| `list_options` | — | lists the configurable options with their current values |
| `set_option` | `id`, `value` | sets one option; the change lands in `settings.json` and applies on restart |

Both speak the option **id** — a stable, dotted name:

| id | Kind | Values |
| --- | --- | --- |
| `theme` | enum | `dark`, `light` |
| `shell.program` | string | unset means the platform default login shell |
| `shell.args` | array | |
| `terminal.colors.scheme` | enum | `solarized-dark`, `solarized-light`, `gruvbox-dark`, `gruvbox-light` |
| `terminal.colors.foreground` | string | `"#rrggbb"` |
| `terminal.colors.background` | string | `"#rrggbb"` |
| `terminal.colors.cursor` | string | `"#rrggbb"` |
| `terminal.colors.palette` | array | the 16 ANSI colours — normal 0–7, bright 8–15 |

That is the whole write surface today. The `close`, `sidebar`, `record`,
`open`, `keys`, `terminal.font_size`, `terminal.copy_on_select` and
`terminal.paste_on_right_click` blocks of
[`settings.json`](../reference/settings.md) are file-only — `keys` is
readable as a resource, below.

## Resources

| URI | Holds |
| --- | --- |
| `termherd://options/schema` | the schema of the configurable options |
| `termherd://keys/schema` | the bindable actions, with their default **and** current chords |

`termherd://keys/schema` is generated from the same in-code action table the
keymap itself uses, so it cannot drift from the binary you are running. It is
the machine-readable form of
[Keyboard shortcuts](../reference/keyboard.md), and the catalogue the live
bridge's `run_action` speaks.

## What it is not

It does not reach the running app: no sessions, no tabs, no terminals, no
keyboard. That is the [live bridge](./live-bridge.md), and it exists only
inside a session TermHerd launched.

It is also **not** `F-mcp-ide-bridge` — a deferred, unbuilt feature that would
run the other way round, with TermHerd as an MCP *client* of Claude's IDE
bridge. `docs/ARCHITECTURE.md` §15 lists an `mcp` crate as deferred under that
name; the server described here lives in the same repository but answers a
different question.
