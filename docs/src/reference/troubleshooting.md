# Troubleshooting

Each entry gives a symptom, its cause and the fix, then links to the page that
explains it in full. The environment variables named here are listed in
[Environment variables](./environment.md).

## TermHerd will not start a second time

**Symptom.** A second launch exits at once, with code 2, and no window opens.

**Cause.** TermHerd is single-instance. The lock is named after the system temp
directory, so a second launch from the same environment finds it taken. A
launch that carries a `termherd://` link hands the link to the running instance
and exits with code 0 instead.

**Fix.** Use the window already open. To run a second build beside it — a
build from source, a worktree — give that build a throwaway temp directory
(`TMPDIR` on macOS and Linux, `TMP` on Windows), so its lock has another name.
The commands are in
[Installation](../guide/installation.md#where-the-log-goes).

## Where is the log?

| Platform | The log goes to |
| --- | --- |
| macOS, Linux | stderr: start TermHerd from a terminal to read it |
| Windows, release build | `%USERPROFILE%\.termherd\termherd.log`, even when started from a terminal |
| any OS, debug build from source | the terminal it was started from |

On Windows the log is renamed `termherd.log.old` at the first launch after it
reaches 5 MB, and a fresh one starts. `RUST_LOG` sets what is logged, for
example `RUST_LOG=termherd=debug`. Details:
[Where the log goes](../guide/installation.md#where-the-log-goes).

## A console window opens beside TermHerd on Windows

**Symptom.** Started from Explorer or the Start menu, TermHerd opens a black
console window as well as its own; closing the console closes TermHerd.

**Cause.** TermHerd asks Windows not to allocate a console when nothing hands
it one. Only Windows 11 24H2 and later read that request; older versions
ignore it.

**Fix.** Leave the console open, or update Windows. Started from a terminal,
TermHerd uses that terminal's console on every version.

## A notification click does nothing

**Symptom.** Clicking a desktop notification does not bring the session
forward.

| Platform | Cause | Fix |
| --- | --- | --- |
| Windows | Once a toast times out into the action centre, its click is lost. | Click the toast while it is still on screen. |
| Linux | The notification server must support actions; some do not. | Use a notification server that supports actions. |
| Linux, Wayland | The compositor may refuse to raise the window. | Switch to TermHerd by hand; the right tab is already active. |
| any | More than 16 notifications wait for a click. | Click or dismiss earlier ones. |

A click also changes nothing but the window when the tab has closed, or while
a prompt is open (a close confirmation, the settings panel, the document
editor).

On Windows a toast is attributed to **PowerShell**, not TermHerd: TermHerd does
not register an application id of its own yet. That is expected, not a sign of
something else posting it.

Details: [Desktop notifications](../workspace/status.md#desktop-notifications).

## A Claude session has no `termherd` MCP server

**Symptom.** `/mcp` in a Claude session lists no `termherd` server, or a
client cannot reach the live bridge.

**Cause.** TermHerd hands the live bridge only to the Claude sessions it
launches itself: a new Claude tab, or a session reopened from the sidebar. It
passes the endpoint on that launch's command line. A `claude` typed by hand
into a shell pane, or started in another terminal, gets nothing.

**Fix.**

- Open the session from TermHerd as a Claude tab.
- To reach the bridge from a client TermHerd did not launch, turn on
  `mcp.attach`, restart TermHerd, and register the client with
  `termherd mcp-config` — see
  [Attaching from outside](../mcp/attach.md#turning-it-on). When that command
  prints nothing, its message on stderr says why:
  [When there is nothing to print](../mcp/attach.md#when-there-is-nothing-to-print).
- With `"attach": "never"` in `settings.json`, `TERMHERD_MCP_ATTACH=1` is
  ignored on purpose: see
  [Refusing it outright](../mcp/attach.md#refusing-it-outright).

To change settings from any Claude session, TermHerd-launched or not, the
[stdio server](../mcp/stdio.md) needs no running app.

## `settings.json` seems ignored

| Symptom | Cause | Fix |
| --- | --- | --- |
| At startup, the sidebar says the file was set aside, and defaults apply. | The file was not valid JSON. It was moved to `settings.json.corrupt-1` (then `-2`, …) so nothing TermHerd saves can overwrite it. | Fix the copy and move it back. |
| An edit made while TermHerd runs changes nothing. | The saved file does not parse; the running settings stay as they were. | Fix the syntax and save again. |
| A number is not the one you wrote. | Out-of-range values clamp: `terminal.font_size` to 6–40, for example. | Use a value inside the range. |
| One key has no effect, the rest do. | A bad value (a typo'd colour, an unknown action name) degrades alone, with a warning in the log. | Read the warning in the [log](#where-is-the-log). |
| A change shows up only later. | Some keys apply at the next session, recording or launch: `shell`, `record`, `mcp.attach`. | Open a new session, start a new recording, or restart. |

The file is strict JSON: no comments, no trailing commas. Details:
[How it loads](./settings.md#how-it-loads).

## A plain shell's status stays on `starting`

**Cause.** The shell reports no prompt marks. TermHerd injects them into zsh
always, and into bash and fish only when `settings.shell` names them; an
unknown shell gets none. On macOS and Linux the foreground process stands in
until the first mark; Windows has no such fallback.

**Fix.** Name your shell in `settings.json`. A zsh whose `~/.zshenv` sets
`ZDOTDIR` loses the marks too: see
[`TERMHERD_ORIG_ZDOTDIR`](./environment.md#termherd_orig_zdotdir). Details:
[Where the signal comes from](../workspace/status.md#where-the-signal-comes-from).
