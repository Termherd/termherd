# The terminal

Each pane is a real PTY running a real program — a login shell, or the Claude
CLI — rendered on a canvas. It is a terminal, not a transcript viewer: full
escape-sequence handling, scrollback, selection, colours.

## Selection and clipboard

Drag with the mouse to select; a double-click selects the word or filename
under the pointer; <kbd>Shift</kbd>+click extends the current selection. The
chords that reach the clipboard:

| | macOS | Windows / Linux |
| --- | --- | --- |
| Copy selection | <kbd>Cmd</kbd>+<kbd>C</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> |
| Paste | <kbd>Cmd</kbd>+<kbd>V</kbd> | <kbd>Ctrl</kbd>+<kbd>V</kbd> or <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> |

Copy/paste is the one binding that is deliberately irregular per platform:
on Windows and Linux <kbd>Ctrl</kbd>+<kbd>C</kbd> must stay the interrupt.
<kbd>Ctrl</kbd>+<kbd>C</kbd> sends `SIGINT` on every platform.

### Mouse gestures

The two classic terminal conventions are available, both **off by default** —
so nothing reaches the clipboard unless you asked for it:

```json
"terminal": { "copy_on_select": true, "paste_on_right_click": true }
```

- **`copy_on_select`** — releasing a drag, or double-clicking a word, copies
  the selection outright. With it off the selection still highlights and waits
  for the copy chord, which reads the highlight currently on screen — so the
  text you copy is the text you just selected, never what you copied last.
- **`paste_on_right_click`** — a right-click pastes into **the pane under the
  pointer**, which need not be the focused one, and is bracketed when that pane
  asked for bracketed paste.

See [`settings.json`](../reference/settings.md).

## Scrollback

The wheel scrolls back through history.
<kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>Up</kbd> jumps to the top of the buffer,
<kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>Down</kbd> back to the bottom.

## Zoom

<kbd>Cmd</kbd>/<kbd>Ctrl</kbd> + <kbd>+</kbd> / <kbd>-</kbd> / <kbd>0</kbd>
steps the font size up, down, and back to the configured base. Zoom is a
runtime state — it does not rewrite `terminal.font_size` in your settings.

Three chords are bound to zoom-in, not one: `mod+=`, `mod+plus` and
`mod+shift+plus`. `=` is the unshifted face of the `+` key on QWERTY and the
unshifted key on AZERTY, so between them the same gesture works across layouts.

## Clickable links

URLs in terminal output (`http`, `https`, `file`, `ftp`) are detected per row.
Hold <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>: the link under the pointer underlines and
the cursor becomes a hand; <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+click opens it in
your OS default handler. Trailing prose punctuation and unbalanced brackets are
trimmed from the match.

## Clickable file paths

The same gesture opens **file paths** — the payoff being that `cargo test`
prints `crates/pty/src/grid.rs:184`, and you click it.

Detection is syntactic, then checked against the disk. A bare word with no
separator and no extension is never a candidate (otherwise every word under the
pointer would cost a `stat`). A `:line[:column]` suffix is split off the target
before resolution. URL cells are masked out so the two detectors never light
the same columns.

Resolution walks candidate roots **innermost first**, and that order *is* the
disambiguation rule — `cargo`, `git` and `pytest` print relative to different
roots:

1. the session's live working directory (which follows your `cd`s),
2. the repository containing it,
3. the directory the session was launched from.

**A path underlines only once it has resolved.** An underline that turns out to
point at nothing is worse than an underline one frame late.

### What it opens the file *with*

By default, the OS default handler — and that has two consequences worth
knowing, both of which the `open` setting removes.

**A line number cannot be honoured.** Detection splits `:184` off the target
and carries it all the way through, but "open this file" is all the OS handoff
can express.

**Executable-by-association files are neither underlined nor opened.** Handing
a path to the OS means "do what the association says", and for a program that
means run it — an `ls` of an untrusted clone is enough to put `payload.app` on
screen. The refusal happens where the resolved path arrives, so hover and click
can never disagree: what will not open does not underline either, and you see
the refusal before you click. It is a mitigation, not a guarantee: on macOS and
Linux the set of extensions that execute is small and closed, but on Windows
the association table maps `.js`, `.py` and every installed language onto an
interpreter, and excluding those would refuse exactly the source files the
feature exists to open.

**Naming an editor removes both.** Set `open.command` with `{path}`, `{line}`
and `{col}` templates — see [`settings.json`](../reference/settings.md) — and
the click lands on the right line. An explicit editor consults no association
either, so the refusal above lifts and those files open like any other. Give a
**GUI** editor: the child's standard streams are closed, so a terminal editor
would start invisible and unkillable. The command runs as argv, never through a
shell, and it is split on whitespace *before* `{path}` is filled in — so a
filename containing spaces or `&` can never become a second argument.

One limit remains, known and accepted: a path that wraps across two lines is
not detected, the same limit URLs have.

## Colours and font

The terminal grid keeps its own colours, independent of the app chrome theme.
Start from a built-in scheme (`solarized-dark`, `solarized-light`,
`gruvbox-dark`, `gruvbox-light`) and override any slot — foreground,
background, cursor, and the 16 ANSI colours. A malformed colour degrades alone,
with a logged warning, instead of failing the file. See
[`settings.json`](../reference/settings.md).

## What the shell announces

TermHerd tracks the session's working directory from the shell's own OSC 7
announcements, so a pane's `cwd` follows your `cd`s rather than reporting the
directory it was launched in forever. That live `cwd` is what
<kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>T</kbd> opens a new shell in, what path
resolution tries first, and what `snapshot` reports over
[MCP](../mcp/index.md).
