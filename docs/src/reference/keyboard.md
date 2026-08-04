# Keyboard shortcuts

Every shortcut is an **action** with a kebab-case name and a default chord.
Rebind any of them in the `keys` section of
[`settings.json`](./settings.md); an override **replaces** that action's
default, and unlisted actions keep theirs.

Below, **`mod`** is the platform primary modifier: <kbd>Cmd</kbd> on macOS,
<kbd>Ctrl</kbd> everywhere else. It is a shorthand for reading this table —
**not** valid chord syntax. Write concrete modifiers in your own bindings.

## The full action vocabulary

### Tabs

| Action | Default | Does |
| --- | --- | --- |
| `next-tab` | `ctrl+tab` | next tab (Ctrl on every platform) |
| `prev-tab` | `ctrl+shift+tab` | previous tab |
| `activate-tab-1` … `-9` | `mod+1` … `mod+9` | jump to tab N |
| `new-shell-here` | `mod+t` | new shell in the focused session's directory |
| `new-claude-session-here` | `mod+alt+t` | new Claude session in that directory |
| `reopen-closed-tab` | `mod+shift+t` | reopen the tab you just closed |
| `close-focused` | `mod+w` | close the focused pane; a lone pane closes its tab |
| `open-new-session` | *(unbound)* | reserved — no surface yet |

`activate-tab-N` is matched by **physical key position**, so it lands on the
same keys on AZERTY and QWERTZ, where the number row produces `&`, `é`, …
without Shift.

### Splits and focus

| Action | Default | Does |
| --- | --- | --- |
| `split-vertical` | `mod+d` | split side by side |
| `split-horizontal` | `mod+shift+d` | split stacked |
| `focus-left` | `mod+shift+left` | focus the pane to the left |
| `focus-right` | `mod+shift+right` | … to the right |
| `focus-up` | `mod+shift+up` | … above |
| `focus-down` | `mod+shift+down` | … below |
| `focus-next` | *(unbound)* | cycle forward through panes |
| `focus-prev` | *(unbound)* | cycle backward |

### Terminal

| Action | Default (macOS) | Default (Windows / Linux) |
| --- | --- | --- |
| `copy` | `cmd+c` | `ctrl+shift+c` |
| `paste` | `cmd+v` | `ctrl+v`, `ctrl+shift+v` |
| `scroll-top` | `cmd+up` | `ctrl+up` |
| `scroll-bottom` | `cmd+down` | `ctrl+down` |
| `zoom-in` | `cmd+=`, `cmd+plus`, `cmd+shift+plus` | `ctrl+…` (same three) |
| `zoom-out` | `cmd+-` | `ctrl+-` |
| `zoom-reset` | `cmd+0` | `ctrl+0` |

Copy/paste is the one pair whose default is irregular per platform: on Windows
and Linux <kbd>Ctrl</kbd>+<kbd>C</kbd> must stay the interrupt, so copy takes
Shift. <kbd>Ctrl</kbd>+<kbd>C</kbd> sends `SIGINT` everywhere.

Zoom-in binds three chords because `=` is the unshifted face of the `+` key on
QWERTY and the unshifted key on AZERTY; between them the same gesture works
across layouts.

### App

| Action | Default | Does |
| --- | --- | --- |
| `focus-search` | `mod+f` | focus the sidebar search box |
| `toggle-sidebar` | `mod+b` | show / hide the sidebar |
| `capture` | `mod+shift+s` | write a state dump + screenshot |
| `toggle-record` | `mod+shift+r` | start / stop a GIF screencast |

### Not in the keymap

| Gesture | Does |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>C</kbd> | interrupt (`SIGINT`) — passed through to the program |
| <kbd>Escape</kbd> | cancel an open prompt, rename or doc pane |
| <kbd>Enter</kbd> | confirm an open prompt |
| Drag a selection | select; copies too with `terminal.copy_on_select` (off by default) |
| Right-click | paste, with `terminal.paste_on_right_click` (off by default) |
| Wheel | scroll back through history |
| <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+click | open a URL or file path under the pointer |
| Drag a tab | reorder it |

<kbd>Escape</kbd> and <kbd>Enter</kbd> are bound to no *action* on purpose:
they belong to whichever overlay is open. That is also what makes them the only
way an [MCP caller](../mcp/keyboard.md) can answer a prompt it armed.

## Chord syntax

Case- and order-insensitive. Modifiers `ctrl`, `shift`, `alt`, `cmd`, joined to
a key with `+`:

```json
"keys": {
  "copy": "ctrl+y",
  "paste": ["ctrl+shift+v", "shift+insert"],
  "activate-tab-1": "alt+1"
}
```

One chord or a list of chords per action. Unknown action names and unparsable
chords are logged and skipped — they do not invalidate the rest of the file.

## Reading the live keymap

The [stdio MCP server](../mcp/stdio.md) publishes the whole catalogue —
every action with its default *and* current chords — as a resource at
`termherd://keys/schema`. It is generated from the same in-code table this page
describes, so it cannot drift from the binary you are running.
