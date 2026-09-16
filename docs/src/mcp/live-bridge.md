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
| `list_sessions` | — | `{ sessions: [...] }` — each row a live session: stable `handle`, tab title, cwd, kind (`shell` / `claude`), resumed Claude id, status |
| `snapshot` | `sections`, `terminals`, `text_lines` | the whole state: config, sidebar, tabs and panes |
| `read_terminal` | `session`, `lines` | `{ text, rendered }` |
| `screenshot` | `max_width` | the window as a PNG |

Every tool answers a JSON **object**: MCP clients reject anything else on their
schema check, which is why `list_sessions` puts its rows in a `sessions` field
rather than answering the array itself.

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
| `mouse_in_session` | `session`, `kind`, `col`, `row`, `button` | a mouse event at a **cell** of the terminal; see below |
| `add_repo` | `path` | put a repository in the sidebar before it has any session |
| `forget_repo` | `path` | drop an addition; the row survives on its sessions |

Each returns the resulting `focused_handle` (`null` when the workspace is now
empty).

The two repo tools answer about a **sidebar row** rather than about focus, so
they add four fields:

| Field | Means |
| --- | --- |
| `repo_path` | the **normalised** key the row is filed under |
| `declared` | whether it is currently a hand-added repository |
| `session_count` | sessions on that row right now |
| `in_sidebar` | whether a row is there at all |

The last two report **membership**, not what the window happens to be drawing:
a search left in the box, or the archived filter, changes neither. Otherwise a
successful `add_repo` would read back as a failure for no reason the caller
could see.

`repo_path` is the one to keep. `add_repo` files a path by **exactly the rule
the scan uses** for a session's working directory — a worktree collapses onto
its main checkout, a file becomes its parent directory, everything else is kept
as given (symlinks included, and *not* climbed to a repository root). Two
spellings of one directory are one key: a trailing slash, a `./`, and forward
slashes on Windows all normalise away, since none of them is a spelling the
scan can produce. That agreement is what stops one repository from occupying
two rows, so address the row afterwards with what came back, not with what you
sent. A path that does not exist, or a relative one, is rejected.

`forget_repo` is the asymmetric one: forgetting a repository that was never
added is **not** an error, and forgetting one the scan still reports leaves the
row standing. Read `in_sidebar` to tell the two outcomes apart — `false` means
it is gone, `true` with `declared: false` means it lives on its sessions.

#### The pointer, inside a terminal

`mouse_in_session` is the pointer counterpart of `run_in_session`: it places
one mouse event **inside** a session's terminal. It is addressed by cell, not
by pixel — a terminal is a grid, and a grid is what a mouse report carries —
so an agent with no screen coordinates can still point.

| Arg | Values |
| --- | --- |
| `kind` | `press`, `release`, `click`, `drag`, `move` |
| `col`, `row` | 0-based cells of the **visible** screen |
| `button` | `left` (default), `middle`, `right` |

The answer adds a `pointer` field saying what the terminal did:

| `pointer` | Means |
| --- | --- |
| `forwarded` | the program in the session reads the mouse and was sent the event |
| `selection` | no program reads the mouse; the event drove the terminal's own text selection |
| `ignored` | it drove nothing — a release, a move or a non-left button with no program reading the mouse, or a motion the program's mouse mode does not cover |

Which of the first two you get is the **program's** choice, not yours. A
full-screen program that turns mouse reporting on — Claude Code's `/diff` and
`/resume`, vim, lazygit, fzf, less — owns the mouse while it runs: every event
goes to it in the encoding it negotiated, and the terminal selects nothing of
its own. Follow a `forwarded` with `wait_for_status` / `read_terminal` to see
what the program made of it, as after `run_in_session`. Mouse reporting comes
in three widths, and a motion the program did not ask for is dropped rather
than selected: click-only reporting takes presses and releases, drag reporting
adds motion with a button held, and motion reporting takes every move.

At a plain shell, or any program not reading the mouse, the same calls drive
the terminal's selection. A drag is two calls — `press` at one cell, then
`drag` at another — and the text between them is selected, both cells
included. Read it back with the `copy` action (`run_action`), which puts the
selection on the clipboard. A bare `click` clears the selection. A cell outside
the pane's geometry, or a session that has not rendered yet, **rejects the
whole call** before anything applies, naming the geometry so you can retry
inside it.

The report carries no modifier keys: a `press` is a plain press whatever the
human's keyboard is doing. The same split governs a human's mouse over the
pane — see [When the program reads the mouse](../workspace/terminal.md#when-the-program-reads-the-mouse).

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
  caller cannot tell how far it got. A pointer event outside the pane, or an
  unknown pointer word, is refused the same way.
- A wedged shell surfaces as a tool error, never a hang.

## What is still open

Five follow-ups, and they are independent of each other:

| Gap | Issue |
| --- | --- |
| The composed prompt → wait → read in **one** round trip. Today you compose it yourself from the three calls above. | [#196](https://github.com/Termherd/termherd/issues/196) |
| `enter` commits neither rename over MCP — see [Driving the keyboard](./keyboard.md). | [#246](https://github.com/Termherd/termherd/issues/246) |
| The doc editor discards unsaved edits when it closes, by button or by `escape`. | [#248](https://github.com/Termherd/termherd/issues/248) |
| The bridge is reachable only from a session termherd spawned, so the launcher itself cannot drive it — see [Two surfaces](./index.md). | [#267](https://github.com/Termherd/termherd/issues/267) |
| No pointer at TermHerd's own interface: the sidebar, the tab strip, a split gutter. | [#301](https://github.com/Termherd/termherd/issues/301) |
