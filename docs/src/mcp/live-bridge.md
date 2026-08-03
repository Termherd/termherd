# The live bridge

A Claude session **launched from TermHerd** is wired to an in-process MCP
server on loopback, with a per-session token, injected into its `mcpServers` at
spawn. Nothing to configure — if you started the session from the app, the
tools are there.

Its private files — the mcp config carrying that bearer token, the
shell-integration directory, the settings overlay — are deleted when the
session is torn down.

## The tools

### Perception

| Tool | Args | Returns |
| --- | --- | --- |
| `list_sessions` | — | every live session: stable `handle`, tab title, cwd, kind (`shell` / `claude`), resumed Claude id, status |
| `snapshot` | `sections`, `terminals`, `text_lines` | the whole state: config, sidebar, tabs and panes |
| `read_terminal` | `session`, `lines` | `{ text, rendered }` |
| `screenshot` | `max_width` | the window as a PNG |

**`snapshot` is light by default**: structure only, no terminal text. Scope
text to named handles with `terminals`, or pass `sections` (any of `"config"`,
`"sidebar"`, `"tabs"`) to narrow it further. `text_lines` defaults to 40. Read
the structure first, then ask for a handle — that ordering is why the filter
exists.

`read_terminal`'s `rendered: false` means the session is live but its screen
has not been drawn yet. **Retry** — do not give up on the handle.

`screenshot` is the pixel companion for render, colour and glyph questions text
cannot answer. Reach for it **last**: a default-bound window is on the order of
200 kB of PNG and a third more again as base64, where a `snapshot` is a few
hundred bytes. `max_width` defaults to 1200 (clamped 64–4096); a total-pixel
ceiling also bounds tall windows the width alone would not, the frame is
area-averaged down rather than nearest-sampled — which is what keeps terminal
glyphs legible at the ~0.4× a retina window is reduced by — and a window
smaller than the bound is never upscaled. The reported `width`/`height` are
what you actually received. A headless run has no window and says so as a
tool-level error; the text reads keep working.

### Action

| Tool | Args | Notes |
| --- | --- | --- |
| `open_session` | `project`, `kind` | `kind` is `"shell"` (default) or `"claude"`; omit `project` for the home dir |
| `split_pane` | `direction`, `pane` | `"vertical"` (default) or `"horizontal"`; omit `pane` for the focused one |
| `focus_pane` | `session` | |
| `rename_tab` | `tab`, `title` | `tab` is the 0-based index `snapshot` reports; a blank title reverts to the derived one |
| `close_pane` | `pane` | a lone pane is its whole tab, which closes |
| `run_in_session` | `session`, `text` | include a trailing newline to submit |

Each returns the resulting `focused_handle` (`null` when the workspace is now
empty).

### Synchronisation

| Tool | Args | Returns |
| --- | --- | --- |
| `wait_for_status` | `session`, `statuses`, `timeout_ms` | `{ status, timed_out }` |

`statuses` defaults to idle-or-attention — the two a caller waiting on a
command actually wants. `timeout_ms` defaults to 30 000 and is capped at
300 000.

**A timeout is not an error.** On expiry the reported `status` is the session's
current one, and `timed_out` is `true`. And a session that **exits** settles
the wait whatever you asked for — it can no longer reach your target. Both
behaviours exist so a wait can never silently park you.

### The keyboard

`press_keys` and `run_action` drive TermHerd's own interface — see
[Driving the keyboard](./keyboard.md).

## The loop: act → wait → observe

`run_in_session` **returns as soon as the text is sent**. It does not wait for
the command.

```text
run_in_session(session, "cargo test\n")
        │
        ▼
wait_for_status(session, ["idle", "attention"])
        │
        ▼
read_terminal(session, lines: 60)
```

**Do not poll `snapshot` in a loop.** It races the transition you are watching
for — that race is exactly why the wait tool exists.

A worked example, from inside a session TermHerd launched:

```text
1. split_pane({ direction: "vertical" })     → focused_handle: "7"
2. run_in_session({ session: "7",
                    text: "cargo test --workspace\n" })
3. wait_for_status({ session: "7",
                     timeout_ms: 300000 })   → { status: "idle",
                                                 timed_out: false }
4. read_terminal({ session: "7", lines: 80 })
```

## Errors and refusals

- An unknown handle, an out-of-range tab index, a non-numeric handle → an
  `invalid_params` error naming the problem.
- A malformed chord or unknown action name **rejects the whole call** before
  anything applies: half an applied sequence is worse than none, because the
  caller cannot tell how far it got.
- A wedged shell surfaces as a tool error, never a hang.

## What is still open

`F-mcp-agent-loop` ([#196](https://github.com/Termherd/termherd/issues/196)) —
the composed prompt → wait → read in **one** round trip — is the remaining
piece. Today you compose it yourself from the three calls above.
