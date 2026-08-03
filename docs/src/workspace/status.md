# Status and attention

Running many sessions is only useful if you can tell, at a glance, which one
needs you. Every session carries one of five activity statuses:

| Status | Means |
| --- | --- |
| `starting` | spawned, nothing heard from it yet |
| `busy` | running a command / thinking |
| `idle` | at a prompt, waiting for input |
| `attention` | actively wants you — a permission prompt, a question |
| `exited` | the program is gone |

The status drives the **tab activity dot**, the focused-terminal badge, the
close confirmation, and the MCP [`wait_for_status`](../mcp/live-bridge.md)
tool. One value, four readers.

## Where the signal comes from

Status is folded from what the terminal says about itself — not guessed from
output text.

**Claude sessions** report through the CLI's OSC title stream. Two things are
needed for that stream to exist, and TermHerd arranges both:

- The CLI only emits its busy/idle/attention stream when it detects iTerm2 as
  the host, so the PTY adapter advertises `TERM_PROGRAM=iTerm.app` (with a
  version) on every spawned session.
- A `CLAUDE_CODE_DISABLE_TERMINAL_TITLE` in your own `~/.claude/settings.json`
  would silence it entirely, and that `env` block outranks the environment
  TermHerd spawns with. So a Claude launch passes a private `--settings`
  overlay, which outranks it in turn and *merges with* — never replaces — your
  settings. This is why the CLI floor is **1.0.61**.

The CLI's own product name (`✳ Claude Code`), which it reports until it has
something session-specific to say, is ignored: a tab would otherwise trade its
project name for the program's.

**Plain shells** get an injected **OSC 133 shell-integration snippet**, whose
prompt and command marks fold into the same status:

| Shell | How | Always on? |
| --- | --- | --- |
| zsh | a private `ZDOTDIR` | ✅ yes |
| bash | `--rcfile` | only when named in `settings.shell` |
| fish | `--init-command` | only when named in `settings.shell` |

Each recipe replays *your* startup files first, so your prompt and aliases are
untouched.

Bash and fish need their snippet as a **command-line argument**, and TermHerd
will not add one to the platform's *default* program — that would demote your
login shell to an ordinary one and change which startup files run. So: name
your shell explicitly in `settings.json` to get the finer marks. zsh needs no
argument and is always integrated.

**Where injection cannot apply** — an unknown shell, an unwritable temp
directory — the PTY's **foreground process group** stands in. It is retired for
good by the first real mark or Claude signal, so it can never contradict what
the terminal says about itself. Windows ConPTY exposes no foreground process
group, so a shell with neither route stays on `starting` there.

## Close confirmation

Closing is governed per action — `tab` and `app` — by one of three policies:

| Policy | Behaviour |
| --- | --- |
| `alwaysConfirm` | always ask |
| `confirmWhenActive` | **default** — ask only while a session runs a foreground process |
| `noConfirmation` | never ask |

Under the default, "runs a foreground process" means `busy` or `attention`.
That is why the status work matters beyond the badges: while every shell sat on
`starting`, a shell running a long command closed without asking.

Quitting names the cost: *"Quit TermHerd? 3 open session(s) will be
force-stopped — any running work is lost."* Every non-exited session dies on
quit, Claude or shell.

Every confirmation is answerable from the keyboard — <kbd>Escape</kbd> to
cancel, <kbd>Enter</kbd> to confirm — which is also what makes them reachable
from [MCP](../mcp/keyboard.md).
