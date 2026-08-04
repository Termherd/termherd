# `settings.json`

```text
~/.termherd/settings.json                     (macOS, Linux)
%USERPROFILE%\.termherd\settings.json         (Windows)
```

There is **no in-app settings panel yet**: edit the file and restart.

The annotated template with every option, its default and its meaning is
[`docs/settings.example.jsonc`](https://github.com/Termherd/termherd/blob/main/docs/settings.example.jsonc)
— copy the blocks you want and strip the comments. **The real file is strict
JSON**: no comments, no trailing commas.

## How it loads

Read once at startup, and defensively:

- Every field is optional. A missing file, a missing field, or a corrupt file
  falls back to built-in defaults — settings never block startup.
- Out-of-range values **clamp** instead of failing the file.
- One bad value — a typo'd colour, an unknown action name — **degrades alone**
  with a logged warning. The rest of the file still applies.

Two neighbouring files are TermHerd's, not yours to edit: `window.json` (size
and position — a position left off every connected monitor is dropped, so the
window re-centers instead of opening out of reach) and `metadata.json` (stars,
archives, custom titles).

## Options

### `shell`

The program launched for each session. Omit the block, or set it to `null`, for
the platform default login shell. `args` is optional.

```json
"shell": { "program": "pwsh", "args": [] }
```

Naming your shell here has a side effect worth knowing: bash and fish get their
OSC 133 shell-integration snippet — and therefore an accurate activity status —
only when named explicitly. See [Status and attention](../workspace/status.md).

### `theme`

`"dark"` (default) or `"light"`. GUI chrome only — sidebar, tab strip, buttons.
The terminal grid keeps its own colours.

### `close`

Per-action close confirmation. Both keys default to `"confirmWhenActive"`.

```json
"close": { "tab": "confirmWhenActive", "app": "confirmWhenActive" }
```

| Value | Behaviour |
| --- | --- |
| `alwaysConfirm` | always ask |
| `confirmWhenActive` | ask only while a session runs a foreground process |
| `noConfirmation` | never ask |

### `terminal`

```json
"terminal": {
  "font_size": 14,
  "copy_on_select": false,
  "paste_on_right_click": false,
  "colors": {
    "scheme": "solarized-dark",
    "foreground": "#839496",
    "background": "#002b36",
    "cursor": "#839496",
    "palette": ["#073642", "#dc322f", "…16 entries…"]
  }
}
```

| Key | Default | Notes |
| --- | --- | --- |
| `font_size` | `14` | pixels, clamped to 6–40. The zoom chords step from here at runtime without rewriting it. |
| `copy_on_select` | `false` | a drag release or a double-click copies the selection outright, no chord needed |
| `paste_on_right_click` | `false` | a right-click pastes into the pane under the pointer, not necessarily the focused one |
| `colors.scheme` | built-in | `solarized-dark`, `solarized-light`, `gruvbox-dark`, `gruvbox-light` |
| `colors.foreground` / `.background` / `.cursor` | from the scheme | `"#rrggbb"`; the `#` is optional |
| `colors.palette` | from the scheme | the 16 ANSI colours — normal 0–7, then bright 8–15 |

Every colour field is optional and overrides the scheme it starts from. Fewer
than 16 palette entries override the head of the list; entries past 16 are
ignored.

### `sidebar`

```json
"sidebar": { "session_limit": 5 }
```

Sessions shown per project before the tail folds behind a `… N more` expander.
`0` shows every session.

### `record`

The GIF screencast budget
([<kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd>](../workspace/capture.md)).
Values clamp: fps 1–60, `max_seconds` 1–600, `scale` 0.1–1.0.

```json
"record": { "fps": 8, "max_seconds": 30, "scale": 0.5 }
```

### `open`

The command a <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>-clicked file path opens in. Omit
the block to hand the file to the OS default handler (`open` / `explorer` /
`xdg-open`) — the default, which cannot honour a line number.

```json
"open": { "command": "code -g {path}:{line}:{col}" }
```

Templates: `{path}` (required), `{line}`, `{col}`. A path the terminal printed
without a position opens at 1:1, so one command stays well-formed either way.

| Rule | Why |
| --- | --- |
| Give a **GUI** editor | the child's standard streams are closed, so `vim` would start invisible and unkillable. Reach one through a terminal emulator: `["wezterm", "start", "--", "vim", "+{line}", "{path}"]` |
| Run as argv, never a shell | the string is split on whitespace **before** `{path}` is filled in, so a filename with spaces or `&` can never become a second argument. Use the array form when the program's own path has spaces |
| No `~`, no shell lookup | the program is spawned directly; write the full path. An app launched from Finder/Explorer inherits a minimal `PATH` that often lacks `code` |
| Windows: `code.cmd` | a direct spawn only ever appends `.exe`, so a bare `"code"` is not found |
| A placeholder in the *program* name is refused | what the terminal printed chooses the file, never the executable |

An empty command, one without `{path}`, or one with an unknown placeholder is
logged and ignored — the OS handoff applies instead. A command that fails to
start raises a desktop notification.

Configuring this also **lifts a restriction**: with no command, a path the OS
would *run* rather than show (`.app`, `.exe`, `.desktop`) is neither underlined
nor opened, because the handoff obeys the file association. An explicit editor
consults no association, so those files open like any other.

### `keys`

Keyboard overrides — one chord or a list per action, **replacing** that
action's default. The full vocabulary and its defaults are in
[Keyboard shortcuts](./keyboard.md).

```json
"keys": {
  "copy": "ctrl+y",
  "paste": ["ctrl+shift+v", "shift+insert"],
  "activate-tab-1": "alt+1"
}
```

## Reading and writing it from a Claude session

The [stdio MCP server](../mcp/stdio.md) exposes a **subset** of these options
as `list_options` / `set_option`, so you can ask "what can I configure here?"
or "switch me to a light theme" from any Claude session.

The eight ids it covers today: `theme`, `shell.program`, `shell.args`,
`terminal.colors.scheme`, `terminal.colors.foreground`,
`terminal.colors.background`, `terminal.colors.cursor`,
`terminal.colors.palette`. The `close`, `sidebar`, `record`, `open`, `keys`,
`terminal.font_size`, `terminal.copy_on_select` and
`terminal.paste_on_right_click` blocks are file-only for now; `keys` is
published as a read-only resource.

A `set_option` write lands in `settings.json` and **applies on restart**, like
any other edit to the file.

## A complete example

```json
{
  "shell": { "program": "/bin/zsh" },
  "theme": "dark",
  "close": { "tab": "confirmWhenActive", "app": "alwaysConfirm" },
  "terminal": {
    "font_size": 15,
    "copy_on_select": true,
    "paste_on_right_click": true,
    "colors": { "scheme": "gruvbox-dark" }
  },
  "sidebar": { "session_limit": 0 },
  "record": { "fps": 10, "max_seconds": 20, "scale": 0.5 },
  "open": { "command": "code -g {path}:{line}:{col}" },
  "keys": {
    "toggle-sidebar": "ctrl+alt+b",
    "focus-next": "ctrl+alt+right"
  }
}
```
