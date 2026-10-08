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
| `list_sessions` | — | `{ sessions: [...] }` — each row a live session: stable `handle`, tab title, cwd, kind (`shell` / `claude`), resumed Claude id (`resume_id`, null for a fresh tab — `session_id` is the live one), status, and Claude's `pid`, `peer_name`, `session_id` |
| `snapshot` | `sections`, `terminals`, `focused_terminal`, `text_lines` | the whole state: config, sidebar, tabs and panes |
| `read_terminal` | `session`, `lines` | `{ text, rendered }` |
| `screenshot` | `max_width` | the window as a PNG |

Every tool answers a JSON **object**: MCP clients reject anything else on their
schema check, which is why `list_sessions` puts its rows in a `sessions` field
rather than answering the array itself.

A tab title carries no kind marker: a fresh tab is titled after its project
alone (`my-app`). Read what a session runs from its `kind` field.

**Which Claude is which.** A session running Claude, in `list_sessions` and in
a `snapshot` pane, also carries `pid`, `peer_name` and `session_id`.
`peer_name` is the name other Claude sessions address it by (`ListAgents` /
`SendMessage`), so two Claude panes in the same directory stay told apart. All
three are read from Claude Code's own `~/.claude/sessions/<pid>.json` for the
job in front of the pane, on every call. That file is the proof a Claude runs
there, whatever the pane's `kind`: a `claude` typed into a shell pane is
identified, and a `vim` left running after Claude quit is not. The file must
also be that process's own: its `procStart` has to match when the process in
front started, so a file a crashed Claude left behind is never pinned on a
program that later reuses its pid. All three are `null` together, and are
always present:

- when no Claude is in front (a shell prompt, any other program);
- when the file in place belongs to an earlier process with the same pid;
- on Windows, which reports no foreground process to take the `pid` from;
- while Claude Code has written no session file, or one without `procStart`
  (an older CLI, a session still starting).

`session_id` is Claude's own id, which changes on a fork or a plan-accept;
address a session by `handle`, never by it.

A `snapshot` pane also carries `color`: the colour Claude Code's `/color` set
for the pane's conversation, spelt as `/color` takes it (`"green"`), or `null`
for a shell, a conversation the sidebar has not listed yet, or one with no
colour of its own. Unlike the three fields above it is read from the
transcript, so it follows the sidebar's rescan rather than each call.

**`snapshot` is light by default**: structure only, no terminal text. Scope
text to named handles with `terminals` (or set `focused_terminal: true` for the
focused pane, when you do not know its handle yet), or pass `sections` (any of `"config"`,
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
| `open_session` | `project`, `kind`, `background` | `kind` is `"shell"` (default) or `"claude"`; omit `project` for the home dir; `background: true` opens without taking focus |
| `split_pane` | `direction`, `pane` | `"vertical"` (default) or `"horizontal"`; omit `pane` for the focused one |
| `focus_pane` | `session` | |
| `rename_tab` | `tab`, `title` | `tab` is the 0-based index `snapshot` reports; a blank title reverts a shell tab to the derived one. A Claude tab is renamed by arming `/rename <title>` for confirmation, answered as `claude_command` is; a blank title, or the name it already shows, arms nothing and leaves it as it is |
| `close_pane` | `pane`, `background` | a lone pane is its whole tab, which closes; `background: true` closes `pane` without focusing it first |
| `run_in_session` | `session`, `text` | include a trailing newline to submit |
| `mouse_in_session` | `session`, `kind`, `col`, `row`, `button` | a mouse event at a **cell** of the terminal; see below |
| `add_repo` | `path` | put a repository in the sidebar before it has any session |
| `forget_repo` | `path` | drop an addition; the row survives on its sessions |
| `claude_command` | `session`, `command`, `argument` | **arms** a confirmation to type a Claude slash command; see below |

Each returns the resulting `focused_handle` (`null` when the workspace is now
empty). `open_session` also returns `opened_handle`, the new session's handle:
read the new session from it rather than from `focused_handle`, which names it
only when the open took focus.

#### Working beside someone who is typing

Every action above moves the keyboard by default: an open activates its new
tab, and a close focuses its target before closing it. An agent orchestrating
workers in the same window as a human would send that human's next keys into a
terminal they did not choose. `background: true` on `open_session` and
`close_pane` keeps the user where they are:

- A background **open** appends the tab at the end of the strip without
  activating it. Into an empty workspace it is the only tab, so it is the
  active one all the same. Its terminal is sized to the tab area at once, so a
  Claude started there draws its first screen at the size it will be shown.
- A background **close** closes `pane` wherever it lives, without revealing
  it, and requires `pane`: the focused pane is the user's, so there is no
  default to fall back on. The flag means *never take focus*, not *focus
  cannot move*: closing the pane that holds focus still hands it to its
  sibling. A lone pane takes its tab with it, onto the reopen stack like any
  tab close, and a close prompt or tab drag the user has under way stays on
  the tab it named.

Neither flag lets an agent reach a state the keyboard cannot: a background tab
is one the user could have opened and then left.

A background session is driven exactly like any other, by handle:
`run_in_session`, `prompt_in_session`, `wait_for_status`, `read_terminal` and
`mouse_in_session` all reach a tab nobody has looked at. The terminal fills its
screen from the program's output, not from being drawn, so `mouse_in_session`
is bounded as soon as the program has printed something.

An MCP close asks no confirmation, background or not: unlike the keyboard's
close, it kills a busy pane straight away. Wait for the session to settle with
`wait_for_status` first when that matters.

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

#### A Claude slash command, confirmed

`claude_command` asks TermHerd to type one of Claude Code's own commands into
a Claude session — the way TermHerd changes what Claude owns, such as a
session's name or colour, rather than keeping a rival copy of it. The list is
closed:

| `command` | `argument` | Types |
| --- | --- | --- |
| `rename` | the new name | `/rename <name>` |
| `color` | `red`, `blue`, `green`, `yellow`, `purple`, `orange`, `pink`, `cyan` or `default` | `/color <colour>` |
| `desktop` | none | `/desktop` |

**Nothing is typed by the call.** It arms the same confirmation prompt the
`send-to-desktop` action arms, naming the exact line, and answers with that
`line` and the prompt's name, `overlay: "claude-command-confirm"`. The prompt
then holds the keyboard: `press_keys(["enter"])` types the line,
`press_keys(["escape"])` drops it, and a human at the window can answer it
too. The prompt is there for the **human at the window**: it shows them the
line before it lands. It does not constrain the caller, which can confirm its
own prompt with `enter` — and could type into the terminal with
`run_in_session` anyway. What the tool adds over raw typing is the checks
below, and one write path shared with the keyboard.

It is **refused**, with nothing armed, when:

- the session is not a Claude launch — a shell would run the line as a
  program;
- Claude is not idle — busy, starting, or waiting on an answer such as a
  permission prompt;
- Claude's prompt holds a **draft** — the error quotes it; the command would
  be typed into it, and a draft of several lines would be submitted with it as
  a prompt to the model;
- Claude's input prompt is **not on screen** — a menu, picker or dialog has the
  keyboard (Enter would pick an entry), or the view is scrolled away from it;
- another prompt is already open.

Confirming checks all of it again against the screen as it is then. A refusal
at that point **keeps the prompt open**, showing why, and `press_keys` reports
the step as `refused` with the reason rather than `overlay`; `escape` dismisses
it, `enter` tries again.

The prompt is read off the screen: the row starting with `❯` under a
horizontal rule, down to the next rule. Its placeholder hint (`Try "…"`) reads
as empty, so a draft spelling exactly that shape is the one case read wrong.

A prompt armed by this tool **ignores a physical Enter for 600 ms**: someone
typing in another pane when it appears would otherwise confirm it with the
Enter that ends their own line. `escape`, and `enter` sent through
`press_keys`, are never held back.

A name is made safe before it is shown: control characters, line breaks and
tabs become spaces, invisible formatting characters are dropped, a trailing
backslash goes (in Claude's prompt, `\` then Enter starts a new line instead of
submitting), and the result is cut to 80 characters on a character boundary
— an accent or a flag is never split. Joiners and variation selectors are kept,
since emoji sequences and Persian or Indic names are spelled with them. A name
with nothing left is refused.

Confirming sends <kbd>Ctrl</kbd>+<kbd>U</kbd> first — a guard against a key
landing between the screen read and the write — then the line, then Enter on
its own.

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
| `prompt_in_session` | `session`, `text`, `statuses`, `lines`, `timeout_ms`, `allow_claude_nesting` | `{ status, timed_out, text, rendered, focused_handle }` — prompt, wait and read in one round trip |

`statuses` defaults to idle-or-attention — the two a caller waiting on a
command actually wants. `timeout_ms` defaults to 30 000 and is capped at
300 000. `prompt_in_session` is the composed agent-loop tool: prompt a session,
wait for its activity status to settle, and read back its terminal text in a
single round trip. Prompting a shell session is enabled by default; prompting
a nested Claude session requires opt-in via `mcp.allow_claude_nesting` setting or
the `allow_claude_nesting: true` parameter.

**A timeout is not an error.** On expiry the reported `status` is the session's
current one, and `timed_out` is `true`. And a session that **exits** settles
the wait whatever you asked for — it can no longer reach your target. Both
behaviours exist so a wait can never silently park you.

### Settings

| Tool | Args | Returns |
| --- | --- | --- |
| `list_options` | — | `{ options }` — every configurable option with its current value |
| `set_option` | `id`, `value` | `{ id, value }` — one writable option, written to `settings.json` |

The same two tools as [the stdio server](./stdio.md), over the same catalogue —
the option ids, which ones are writable, and what each accepts are defined
once and read by both. The write lands in `settings.json`, and the running app
applies it the moment the file changes, as it does for any edit to the file.
A refused value is an `invalid_params` error; a `settings.json` that does not
parse is refused too, rather than overwritten.

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

Alternatively, use `prompt_in_session` to run all three steps in **one** round trip.

**Do not poll `snapshot` in a loop.** It races the transition you are watching
for — that race is exactly why the wait tool exists.

A worked example, from inside a session TermHerd launched:

```text
1. split_pane({ direction: "vertical" })     → focused_handle: "7"
2. prompt_in_session({ session: "7",
                       text: "cargo test --workspace\n",
                       timeout_ms: 300000,
                       lines: 80 })          → { status: "idle",
                                                 timed_out: false,
                                                 text: "...",
                                                 rendered: true,
                                                 focused_handle: "7" }
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

Four follow-ups, and they are independent of each other:

| Gap | Issue |
| --- | --- |
| `enter` commits neither rename over MCP — see [Driving the keyboard](./keyboard.md). | [#246](https://github.com/Termherd/termherd/issues/246) |
| The doc editor discards unsaved edits when it closes, by button or by `escape`. | [#248](https://github.com/Termherd/termherd/issues/248) |
| The bridge is reachable only from a session termherd spawned, so the launcher itself cannot drive it — see [Two surfaces](./index.md). | [#267](https://github.com/Termherd/termherd/issues/267) |
| No pointer at TermHerd's own interface: the sidebar, the tab strip, a split gutter. | [#301](https://github.com/Termherd/termherd/issues/301) |
