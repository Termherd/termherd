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
| `rename-tab` | `mod+shift+i` | rename the focused tab, as a double-click does |
| `open-tab-menu` | `mod+shift+m` | open the focused tab's menu, as a right-click does |
| `open-new-session` | *(unbound)* | reserved — no surface yet |

`rename-tab` opens the same inline field a double-click on the tab opens,
filled with the tab's current name: <kbd>Enter</kbd> keeps the edit — on a
Claude tab by asking Claude to `/rename` it, as
[a double-click does](../workspace/tabs-and-splits.md) — and
<kbd>Escape</kbd> drops it. Its chord is Terminal.app's *Edit Title* on macOS;
on Windows and Linux <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> reaches a
program in the terminal as the same byte as <kbd>Ctrl</kbd>+<kbd>I</kbd>, which
is <kbd>Tab</kbd>, so claiming it takes nothing away from that program.

`open-tab-menu` opens the [tab menu](../workspace/tabs-and-splits.md#tabs) a
right-click opens; every entry in it is one of the actions on this page. With
no tab open it does nothing. The chord follows the letter M wherever the layout
puts it. On Windows and Linux <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>M</kbd>
reaches a terminal program as <kbd>Ctrl</kbd>+<kbd>M</kbd>, which is
<kbd>Enter</kbd>, so the unshifted chord still gets there.

**Bind letters and named keys, not punctuation that needs Shift.** A chord is
matched on the key pressed *before* Shift applies, so on French AZERTY, where
`.` is <kbd>Shift</kbd>+<kbd>;</kbd>, a press of
<kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>;</kbd> is the chord `cmd+shift+;`, never
`cmd+shift+.`. When a chord does nothing, run with
`RUST_LOG=termherd=debug`: every chord with a modifier other than Shift that
reaches no action is logged with the key and modifiers it arrived as.

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
| `copy-agent-name` | *(unbound)* | *(unbound)* |
| `send-to-desktop` | *(unbound)* | *(unbound)* |
| `scroll-top` | `cmd+up` | `ctrl+up` |
| `scroll-bottom` | `cmd+down` | `ctrl+down` |
| `zoom-in` | `cmd+=`, `cmd+plus`, `cmd+shift+plus` | `ctrl+…` (same three) |
| `zoom-out` | `cmd+-` | `ctrl+-` |
| `zoom-reset` | `cmd+0` | `ctrl+0` |

Copy/paste is the one pair whose default is irregular per platform: on Windows
and Linux <kbd>Ctrl</kbd>+<kbd>C</kbd> must stay the interrupt, so copy takes
Shift. <kbd>Ctrl</kbd>+<kbd>C</kbd> sends `SIGINT` everywhere.

`copy-agent-name` copies the peer name of the Claude in the focused pane:
the name `/list-agents` shows and another Claude session addresses it by, such
as `termherd-b0`. It reads Claude's session file at the moment you press it,
and does nothing when no Claude runs there or it has not written one yet.
Windows reports no foreground process to a terminal, so there it never finds
one. Bind it yourself; it has no default.

`send-to-desktop` asks to type `/desktop` into the Claude in the focused pane,
which hands the session to the Claude desktop app. A prompt shows the exact line
first: <kbd>Enter</kbd> types it, <kbd>Escape</kbd> drops it. It does nothing
unless that Claude is idle with nothing typed in its prompt. Bind it yourself;
it has no default.

Zoom-in binds three chords because `=` is the unshifted face of the `+` key on
QWERTY and the unshifted key on AZERTY; between them the same gesture works
across layouts.

### App

| Action | Default | Does |
| --- | --- | --- |
| `focus-search` | `mod+f` | focus the sidebar search box |
| `toggle-sidebar` | `mod+b` | show / hide the sidebar |
| `open-settings` | `mod+,` | open / close the [settings panel](../workspace/settings-panel.md) |
| `capture` | `mod+shift+s` | write a state dump + screenshot |
| `toggle-record` | `mod+shift+r` | start / stop a GIF screencast |

### Not in the keymap

| Gesture | Does |
| --- | --- |
| <kbd>Ctrl</kbd>+<kbd>C</kbd> | interrupt (`SIGINT`) — passed through to the program |
| <kbd>Escape</kbd> | cancel an open prompt, rename, tab menu or doc pane |
| <kbd>Enter</kbd> | confirm an open prompt; run the selected tab-menu entry |
| <kbd>↑</kbd> / <kbd>↓</kbd> in the tab menu | move the selection |
| Right-click a tab | focus it and open its menu |
| Drag a selection | select; copies too with `terminal.copy_on_select` (off by default) |
| Right-click | paste, with `terminal.paste_on_right_click` (off by default) |
| Wheel | scroll back through history, or the wheel event to a program reading the mouse |
| Click, drag, right-click in a program reading the mouse | the event goes to the program — vim, lazygit, Claude Code's `/resume`; nothing is selected or pasted locally |
| <kbd>Shift</kbd>+drag in such a program | the terminal's own selection, as a plain drag is at a shell |
| <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+click | open a URL, hidden (OSC 8) hyperlink or file path under the pointer |
| Drag a tab | reorder it |

<kbd>Escape</kbd> and <kbd>Enter</kbd> are bound to no *action* on purpose:
they belong to whichever overlay is open. That is also what makes them the only
way an [MCP caller](../mcp/keyboard.md) can answer a prompt it armed.

## Chord syntax

Case- and order-insensitive. Modifiers `ctrl`, `shift`, `alt`, `cmd`, joined to
a key with `+`. Aliases: `control` for `ctrl`, `option` for `alt`, and
`super`, `logo`, `win` or `meta` for `cmd`. The `+` key itself is spelled
`plus`, since a literal `+` is the separator:

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

The [stdio MCP server](../mcp/stdio.md) publishes the action catalogue — each
action with its default chords and the override `settings.json` sets for it,
if any — as a resource at `termherd://keys/schema`. It is generated from the
same in-code table this page describes, so it cannot drift from the binary you
are running. Two gaps: the `activate-tab-N` family is not listed, and `copy` /
`paste` show no default, because theirs differ per platform and are set
outside that table — this page has them.
