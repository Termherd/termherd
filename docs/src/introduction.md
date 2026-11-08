# TermHerd

**A terminal workspace for your Claude Code sessions — browse them, launch
them, arrange them in tabs and splits, watch which one needs you, and let a
session drive the workspace it runs in.**

It is a native Rust desktop app (macOS, Windows, Linux — all three
first-class), keyboard-driven, built around a real PTY terminal per session.

```bash
cargo run -p termherd-app     # or install a release build — see Installation
```

## What it is not

**Not an IDE.** TermHerd deliberately does not emulate an editor and does not
register in `~/.claude/ide`: Claude keeps using the editor you already
configured. Dropping IDE emulation is what keeps the product a *workspace* —
the hardest, largest surface (an in-app diff panel) left the critical path, and
what remains is the capability that actually earns its keep: multiplexing many
sessions on one screen.

**Not a replacement for your shell.** A tab can be a Claude session or a plain
shell, and they behave identically. TermHerd arranges terminals; it does not
try to be one you have to relearn.

## What makes it different

**It reads Claude's own files, and never writes to them.** Sessions come from
walking `~/.claude/projects`. Your stars, custom titles, archives and the
repositories you added by hand live in an overlay at
`~/.termherd/metadata.json` — TermHerd never writes under
`~/.claude`. Run it beside another session manager if you want; nothing it does
is destructive to the CLI's own state.

**It knows which session needs you.** Every session carries an activity status
— `starting`, `busy`, `idle`, `attention`, `exited` — folded from what the
terminal says about itself: Claude's OSC title stream, and for a plain shell an
OSC 133 shell-integration snippet TermHerd injects at spawn. The status drives
tab badges, the close confirmation, and the MCP wait tool. It is not a
heuristic on top of output text.

**A session can drive the workspace it lives in.** A Claude session launched
from TermHerd gets an in-process [MCP](https://modelcontextprotocol.io) server
wired into its config at spawn — no setup. It can read the whole workspace,
open and split panes, type into other sessions, wait for one to go idle,
screenshot the window, put a repository in the sidebar, and press TermHerd's
own key chords. That loop is what
lets an agent *verify* a change instead of only proposing one.

**The quality bar is the reason it exists.** TermHerd is a replatform of an
Electron app, and the rewrite is scoped by a fixed list of defects it must fix
*by construction*: a headless, pure domain core (`core::App::apply(Event) ->
Vec<Effect>`), a hexagonal crate graph where adapters depend on the core and
never the reverse, one actor per session instead of shared mutable state, typed
errors with `unwrap`/`panic` clippy-denied in the domain crates, one logging
stack, and CI gates that block a merge. See
[Architecture at a glance](./project/architecture.md).

## Where to go next

| You want to… | Read |
| --- | --- |
| Get it running | [Installation](./guide/installation.md) → [Quick start](./guide/quick-start.md) |
| Learn the workspace | [The sidebar](./workspace/sidebar.md), [Tabs and splits](./workspace/tabs-and-splits.md) |
| Look up a key | [Keyboard shortcuts](./reference/keyboard.md) |
| Change a setting | [`settings.json`](./reference/settings.md) |
| Let an agent drive it | [Driving termherd over MCP](./mcp/index.md) |
| Understand how it's built | [Architecture at a glance](./project/architecture.md) |

> This book documents shipped behaviour. Where a feature is partial, the page
> says so rather than describing an interface that does not exist yet. The
> authoritative status of every feature is
> [`ROADMAP.md`](https://github.com/Termherd/termherd/blob/main/ROADMAP.md).
