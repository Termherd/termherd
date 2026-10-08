# Tabs and splits

Every session you open is a **tab**. Every tab holds a **pane tree** — one
terminal, or many, split vertically and horizontally.

```text
┌ ●busy ✳ my-app ┬ ○idle ❯ tests ┬ ○idle ❯ notes ────┐
├────────────────┴───────────────┴───────────────────┤
│                        │                           │
│   claude (my-app)      │   $ cargo test            │
│                        │                           │
│                        ├───────────────────────────┤
│                        │                           │
│                        │   $ git log --oneline     │
│                        │                           │
└────────────────────────┴───────────────────────────┘
   mod+D splits vertically · mod+Shift+D horizontally
        (mod = Cmd on macOS, Ctrl on Windows and Linux)
```

## Tabs

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Next / previous tab | <kbd>Ctrl</kbd>+<kbd>Tab</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd> | same |
| Jump to tab 1–9 | <kbd>Cmd</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | <kbd>Ctrl</kbd>+<kbd>1</kbd>…<kbd>9</kbd> |
| New shell here | <kbd>Cmd</kbd>+<kbd>T</kbd> | <kbd>Ctrl</kbd>+<kbd>T</kbd> |
| New Claude session here | <kbd>Cmd</kbd>+<kbd>Alt</kbd>+<kbd>T</kbd> | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>T</kbd> |
| Reopen the tab you closed | <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> |
| Close focused pane | <kbd>Cmd</kbd>+<kbd>W</kbd> | <kbd>Ctrl</kbd>+<kbd>W</kbd> |

**Jump-to-tab is matched by physical key position**, not by the character the
key produces. On AZERTY and QWERTZ, where the number row produces `&`, `é`, …
without Shift, <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>1</kbd> still lands on tab 1.

Each tab carries its own **activity dot** (see
[Status and attention](./status.md)), a **kind mark** — `✳` for a Claude
session, `❯` for a shell — a title derived from its session, and a `×` to close
it. In a split tab, the mark follows the focused pane. The kind is never part
of the title, so renaming a tab or a retitle from Claude cannot lose it: a
fresh tab is titled after its project alone. Hovering a tab shows the
session's fuller description — the card the sidebar shows, for a fresh Claude
tab as soon as the sidebar lists its session. Should `/clear` or a
plan-accept give the conversation a new id, the tab picks it up from
Claude's own session file the next time TermHerd reads it — hovering the
tab, or clicking a row in the sidebar — and keeps it after Claude exits
(not on Windows, where that file cannot be matched to its pane). Until then
the sidebar's status dot stays on the old row. Reopening a closed Claude tab
resumes the conversation it held last, once the sidebar lists it; a fresh
tab closed before then comes back as a new conversation. When a Claude
runs in it, the card also names its agent (`Agent: termherd-b0`), the peer
name other Claude sessions address it by; the `copy-agent-name` action puts
it on the clipboard. In a split tab, the card names the first pane's agent.

**Tabs reorder by drag-and-drop.** Press a tab and drag it onto another slot:
the carried tab fades, the drop slot is outlined, and the reorder commits on
release. The drag survives the pointer leaving the strip: releasing anywhere
drops the tab at the last slot shown. To cancel, drag back onto the carried
tab before releasing; switching away from termherd mid-drag abandons it too.
A plain click still just activates the tab.
The order lives in the pure workspace model — the tab strip holds only
transient pointer state, so there is no second, rival tab tree to drift out
of sync.

## Splits

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Split vertical (side by side) | <kbd>Cmd</kbd>+<kbd>D</kbd> | <kbd>Ctrl</kbd>+<kbd>D</kbd> |
| Split horizontal (stacked) | <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>D</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>D</kbd> |
| Focus a neighbour | <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>←↑↓→</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>←↑↓→</kbd> |

A split opens a **fresh shell** beside the focused pane. Directional focus
walks the pane tree geometrically — <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>→</kbd>
goes to the pane on the right, whatever the nesting.

Closing the **last** pane in a tab closes the tab.

**Drag-resize is not shipped yet.** Panes divide their space evenly; the
remaining piece of `F-terminal-split` is a draggable divider. Two extra focus
actions, `focus-next` and `focus-prev`, exist in the keymap with no default
chord — bind them yourself if you prefer cycling to directional movement
([`settings.json`](../reference/settings.md)).

## Closing, and what asks first

Closing a tab, and quitting the app, are each governed by their own
confirmation policy — `alwaysConfirm`, `confirmWhenActive` (the default), or
`noConfirmation`. Under the default, a tab whose session is mid-command asks
before closing; an idle one closes silently. Quitting names how many sessions
will be force-stopped.

A pane whose **shell exits cleanly closes itself**; one whose shell exited with
a failure stays on screen so you can read what happened.
