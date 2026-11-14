# The terminal

Each pane is a real PTY running a real program — a login shell, or the Claude
CLI — rendered on a canvas. It is a terminal, not a transcript viewer: full
escape-sequence handling, scrollback, selection, colours.

## Selection and clipboard

Drag with the mouse to select; a double-click selects the word under the
pointer, or the whole URL or path when there is one — exactly the span a
[link click](#clickable-links) opens, so a sentence's closing `.` is left out
and a `:42` suffix kept; a triple-click selects the whole line, every row a
long line wrapped onto;
<kbd>Shift</kbd>+click extends the current selection.
The chords that reach the clipboard:

| | macOS | Windows / Linux |
| --- | --- | --- |
| Copy selection | <kbd>Cmd</kbd>+<kbd>C</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> |
| Paste | <kbd>Cmd</kbd>+<kbd>V</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd>; also <kbd>Ctrl</kbd>+<kbd>V</kbd> on Windows |

On Windows and Linux <kbd>Ctrl</kbd>+<kbd>C</kbd> must stay the interrupt; it
sends `SIGINT` on every platform. On Windows <kbd>Ctrl</kbd>+<kbd>V</kbd>
pastes too, as in Windows Terminal, and Claude Code takes its image paste from
<kbd>Alt</kbd>+<kbd>V</kbd>; on Linux <kbd>Ctrl</kbd>+<kbd>V</kbd> reaches the
program, where Claude Code pastes an image on it.

### A paste that would run line by line

A program that turns on *bracketed paste* — Claude Code, zsh, fish, bash 5.1
and later, vim — receives a paste as one block, so its line breaks stay text.
A program that never turned it on, such as `sh` or `cmd.exe`, cannot tell a
paste from typing: each line break is an Enter, and each line runs as it
arrives, including one you did not see on the page you copied from.

So a paste holding a line break — a trailing one included — into such a
terminal waits for you. A prompt names the tab and the number of lines;
**Paste** (or <kbd>Enter</kbd>) sends the text unchanged, **Cancel** (or
<kbd>Escape</kbd>) drops it. The paste chord and the right-click paste both
ask. A single line with no line break, or any paste into a program that turned
bracketed paste on, is written at once.

### Option as Meta (macOS)

The **left** Option key is Meta: <kbd>⌥B</kbd> / <kbd>⌥F</kbd> move by word,
and Claude Code's <kbd>⌥T</kbd> / <kbd>⌥P</kbd> work. The **right** one types
what the layout composes: `|` with <kbd>⌥⇧L</kbd> on a French layout, `å` on
a US one.

```json
"terminal": { "option_as_meta": "left" }
```

`"right"` swaps them, `"none"` makes both compose, `"both"` makes both Meta.
<kbd>⌥Enter</kbd> (a newline in Claude), <kbd>⌥⌫</kbd> and <kbd>⌥←</kbd> /
<kbd>⌥→</kbd> work with either key. On Windows and Linux, Alt is always Meta.

Meta sends `ESC` before what the key would send without it, for any ASCII
character: <kbd>Meta</kbd>+<kbd>.</kbd> is `ESC .` (the shell's last
argument), <kbd>Meta</kbd>+<kbd>Shift</kbd>+<kbd>B</kbd> is `ESC B`,
<kbd>Ctrl</kbd>+<kbd>Meta</kbd>+<kbd>B</kbd> is `ESC ^B`, and
<kbd>Meta</kbd>+<kbd>Space</kbd> is `ESC` and a space. A shifted symbol
types what the layout gives it, without `ESC`.

### Cursor keys

The arrows, <kbd>Home</kbd> and <kbd>End</kbd> follow the mode the program
asked for: `ESC O A` once it switched to application cursor keys (as vim and
less do), `ESC [ A` otherwise. With a modifier held they always send the
`ESC [ 1 ; <mod> A` form.

### Accents and composed text

Dead keys and input methods compose in the focused terminal: <kbd>^</kbd> then
<kbd>e</kbd> types `ê` on a French layout, and CJK input methods work the same
way. While a composition is pending, its text shows underlined over the
cursor cell, at the terminal's font size but in the interface font. It goes
away when the character is typed, and when you press <kbd>Escape</kbd>, leave
the window or open a prompt.

### Mouse gestures

The two classic terminal conventions are available, both **off by default** —
so nothing reaches the clipboard unless you asked for it:

```json
"terminal": { "copy_on_select": true, "paste_on_right_click": true }
```

- **`copy_on_select`** — releasing a drag, or double- or triple-clicking,
  copies the selection outright. With it off the selection still highlights and
  waits for the copy chord, which asks the terminal for its current selection —
  even one scrolled out of view — so the text you copy is the text you just
  selected, never what you copied last. With nothing selected the chord does
  nothing — in particular in a pane whose program owns the mouse (below), where
  the program's own copy of a drag stays on the clipboard untouched.
- **`paste_on_right_click`** — a right-click pastes into **the pane under the
  pointer**, which need not be the focused one, and is bracketed when that pane
  asked for bracketed paste. The click also focuses that pane, so the keys you
  type next go where you just pasted. A confirmation prompt refuses it, exactly
  as it refuses the paste chord — the pointer is not a way past a prompt. A
  multi-line paste into a pane without bracketed paste
  [asks first](#a-paste-that-would-run-line-by-line).

### When the program reads the mouse

A full-screen program that turns mouse reporting on — Claude Code's `/diff`
and `/resume`, vim, lazygit, fzf, less, tmux with `mouse on` — owns the mouse
while it runs. Clicks, drags and (when it asked for them) bare moves go to the
program, so clicking an entry in `/resume` selects it and clicking in vim
places the cursor; the terminal draws no selection of its own over it, and a
right-click reaches the program instead of pasting. The wheel already worked
this way.

Hold <kbd>Shift</kbd> to take the mouse back: <kbd>Shift</kbd>+drag selects
text from a mouse-mode program exactly as a plain drag does at a shell, and the
copy gestures above apply to it. The link modifier does the same for a
<kbd>Ctrl</kbd>/<kbd>Cmd</kbd>+click on a path or URL, which opens it rather
than reaching the program. Neither modifier is reported to the program, so it
never sees the click it lost.

See [`settings.json`](../reference/settings.md).

## Scrollback

The wheel scrolls back through history.
<kbd>Cmd</kbd>+<kbd>Up</kbd> jumps to the top of the buffer and
<kbd>Cmd</kbd>+<kbd>Down</kbd> back to the bottom on macOS;
<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Home</kbd> / <kbd>End</kbd> on Windows
and Linux, where <kbd>Ctrl</kbd>+<kbd>Up</kbd> / <kbd>Down</kbd> reach the
program.

## Zoom

<kbd>Cmd</kbd>/<kbd>Ctrl</kbd> + <kbd>+</kbd> / <kbd>-</kbd> / <kbd>0</kbd>
steps the font size up, down, and back to the configured base. Zoom is a
runtime state — it does not rewrite `terminal.font_size` in your settings.

Three chords are bound to zoom-in, not one: `mod+=`, `mod+shift+=` and
`mod+plus`. On QWERTY and AZERTY `+` is the shifted face of the `=` key, so
zoom-in answers that key with or without Shift; `mod+plus` is the keypad
<kbd>+</kbd>.

## Clickable links

URLs in terminal output (`http`, `https`, `file`, `ftp`, `termherd`) are
detected per row. Hold <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>: the link under the
pointer underlines and the cursor becomes a hand;
<kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+click opens it in your OS default handler.
Trailing prose punctuation and unbalanced brackets are trimmed from the match.
A `termherd://` link comes back to termherd itself: see
[Opening a tab from a link](tabs-and-splits.md#opening-a-tab-from-a-link).

**Hidden hyperlinks** work the same way. A program can print a label over a
URL it never shows — the OSC 8 escape, which `gh`, `ls --hyperlink`, `cargo`
and Claude Code's rendered markdown all emit — so a table cell reading `#76`
may point at the full issue URL. termherd carries that target with the grid:
hover the label and it underlines, click it and the hidden URL opens. Where a
label sits over a hyperlink *and* looks like a URL itself, the hyperlink wins —
it is what the program meant, the text is only its face.

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
not detected, the same limit URLs have; a double-click on it selects only the
part on the clicked line.

## Colours and font

The terminal grid keeps its own colours, independent of the app chrome theme.
Start from one of the 18 built-in schemes, each named like the chrome theme it
matches, and override any slot — foreground,
background, cursor, and the 16 ANSI colours. A malformed colour degrades alone,
with a logged warning, instead of failing the file. See
[`settings.json`](../reference/settings.md).

## What the shell announces

TermHerd tracks the session's working directory from the shell's own OSC 7
announcements, so a pane's `cwd` follows your `cd`s rather than reporting the
directory it was launched in forever. That live `cwd` is what
<kbd>Cmd</kbd>+<kbd>T</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd>
opens a new shell in, what path resolution tries first, and what `snapshot`
reports over
[MCP](../mcp/index.md).

## The environment a pane starts with

A pane inherits TermHerd's environment, with two kinds of change: the
terminal identity Claude's status stream needs (see
[Status](status.md)), and nothing of a Claude Code session TermHerd itself
was started from. Started from a Claude Code shell, TermHerd would otherwise
pass that session's `CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_PID`,
messaging socket and the like to every pane, and a Claude launched there took
itself for part of the parent session and could not be named. Variables you
set yourself, such as `CLAUDE_CONFIG_DIR` or an API key, still reach every
pane.
