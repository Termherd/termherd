# Environment variables

TermHerd reads a few variables from the environment it starts in, and sets a
few on every session it spawns. None is required.

## What TermHerd reads

| Variable | Read for | Notes |
| --- | --- | --- |
| `TERMHERD_MCP_ATTACH` | publishing the live bridge for this launch | `1` / `true` / `yes` / `on` turns `mcp.attach` on, `0` / `false` / `no` / `off` turns it off, case-blind. Anything else is ignored, and so is any value when the setting is `"never"`. See [Attaching from outside](../mcp/attach.md#turning-it-on). |
| `RUST_LOG` | what is logged | For example `termherd=debug`. Where the log goes: [Installation](../guide/installation.md#where-the-log-goes). |
| `TMPDIR` (macOS, Linux), `TMP` (Windows) | the system temp directory | The single-instance lock is named after it, so a build given its own temp directory runs beside the installed one. Each session's private files (the MCP config, shell integration) live there too. |
| `USERPROFILE`, then `HOME` | your home directory | `~/.termherd` and `~/.claude` are found from it. With neither set, nothing is persisted. |
| `TERMHERD_DATA_DIR` | where TermHerd keeps its own files | Replaces `~/.termherd`: settings, layout, log, bridge file. Your home, `~/.claude` and the shells' home are left alone, which is how a second build is kept off your settings without hiding your sessions. `termherd-mcp` reads it too. Empty counts as unset. |
| `CLAUDE_CONFIG_DIR` | where Claude Code keeps its files | Replaces `~/.claude`, the same variable Claude Code reads, so the sidebar lists the sessions the Claude you launch writes. Empty counts as unset. |
| `ZDOTDIR`, `TERMHERD_ORIG_ZDOTDIR` | where a zsh session replays your startup files from | See [below](#termherd_orig_zdotdir). |

## What a session receives

Every session TermHerd spawns, shell or Claude, inherits TermHerd's own
environment with these changes:

| Variable | Value | Why |
| --- | --- | --- |
| `TERM` | `xterm-256color` | a colour-capable terminal type |
| `TERM_PROGRAM` | `iTerm.app` | Claude Code emits its busy / idle / attention stream only when it believes it runs under iTerm2 — see [Status and attention](../workspace/status.md#where-the-signal-comes-from) |
| `TERM_PROGRAM_VERSION` | `3.5.0` | read with `TERM_PROGRAM` as "a recent iTerm2" |
| `ZDOTDIR` | a private per-session directory | zsh only: how the prompt marks are installed |
| `TERMHERD_ORIG_ZDOTDIR` | the directory your own startup files were replayed from | zsh only, and only when TermHerd found one — your `ZDOTDIR`, or else your home |

Some variables are **removed**: the ones a Claude Code session exports to name
*itself* — `CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT`,
`CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_SESSION_ID`,
`CLAUDE_CODE_SESSION_ATTENDED`, `CLAUDE_CODE_MESSAGING_SOCKET`,
`CLAUDE_CODE_MESSAGING_TOKEN`, `CLAUDE_CODE_EXECPATH`, `CLAUDE_PID`,
`CLAUDE_EFFORT` and `AI_AGENT`. A TermHerd started from inside a Claude session
would otherwise hand them on, and each Claude it launched would take itself
for part of that session. The variables you set on purpose, such as
`CLAUDE_CONFIG_DIR` or `CLAUDE_CODE_USE_BEDROCK`, reach every session
unchanged.

The live bridge's address and token are **not** in the environment: a Claude
session receives them in a private file passed with `--mcp-config`.

## `TERMHERD_ORIG_ZDOTDIR`

zsh reads every startup file from `$ZDOTDIR`. To install its prompt marks,
TermHerd points `ZDOTDIR` at a private directory whose `.zshenv`,
`.zprofile`, `.zshrc` and `.zlogin` each source your own file of the same name
first, then (in `.zshrc` only) add the hooks. Your own files are looked for in,
in order:

1. `TERMHERD_ORIG_ZDOTDIR`, when set;
2. `ZDOTDIR`, when set to a directory TermHerd did not generate;
3. `HOME`.

`TERMHERD_ORIG_ZDOTDIR` records the directory that was displaced, so a
TermHerd started from a TermHerd zsh still finds your files rather than the
outer session's private ones.

What this means if you use a custom `ZDOTDIR`:

- **Inside a session, `$ZDOTDIR` is not yours.** It names the private
  directory. A script that needs your configuration directory reads
  `${TERMHERD_ORIG_ZDOTDIR:-$ZDOTDIR}`.
- **Export it before TermHerd starts.** A `ZDOTDIR` in the environment
  TermHerd inherits works: a launch from a terminal whose shell exported it,
  for example.
- **A `ZDOTDIR` set in `~/.zshenv` takes the session over.** Sourcing your
  `~/.zshenv` changes `ZDOTDIR`, and zsh then reads `.zprofile` and `.zshrc`
  from *your* directory instead of the private one. Your configuration loads,
  but TermHerd's hooks never do: the session gets no prompt marks, and its
  status falls back on the foreground process (macOS, Linux). This is what
  happens when TermHerd starts from the Dock, Finder or a desktop launcher,
  whose environment never ran your shell. Starting TermHerd from a terminal
  avoids it.
- **`.zlogout` is not replayed.** zsh reads it from the private directory,
  where there is none.
- **A `ZDOTDIR` typed on a command line inside a session is overridden.**
  `ZDOTDIR=… termherd` started from a TermHerd zsh still replays from
  `TERMHERD_ORIG_ZDOTDIR`, which takes precedence. Unset it first to
  choose another directory.

bash and fish read only `HOME`, and take their hooks as a command-line
argument; neither sets a variable.
