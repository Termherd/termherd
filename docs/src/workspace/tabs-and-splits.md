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
| Rename the focused tab | <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> |
| Open the focused tab's menu | <kbd>Cmd</kbd>+<kbd>.</kbd> | <kbd>Ctrl</kbd>+<kbd>.</kbd> |

**Jump-to-tab is matched by physical key position**, not by the character the
key produces. On AZERTY and QWERTZ, where the number row produces `&`, `é`, …
without Shift, <kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>1</kbd> still lands on tab 1.

Each tab carries its own **activity dot** (see
[Status and attention](./status.md)), a **kind mark** — `✳` for a Claude
session, `❯` for a shell — a title derived from its session, and a `×` to close
it. In a split tab, the mark follows the focused pane. The kind is never part
of the title, so renaming a tab or a retitle from Claude cannot lose it: a
fresh tab is titled after its project alone. Hovering a tab shows the
session's fuller description — the card the sidebar shows. When a Claude
runs in it, the card also names its agent (`Agent: termherd-b0`), the peer
name other Claude sessions address it by; the `copy-agent-name` action puts
it on the clipboard. In a split tab, the card names the first pane's agent.

**Rename a tab** by double-clicking it, or with the `rename-tab` chord for the
focused one. Either opens an inline field holding the tab's current name;
<kbd>Enter</kbd> or a click elsewhere keeps the edit, <kbd>Escape</kbd> drops
it, and an empty name gives the tab back its derived title. The tab strip does
not scroll yet: with more tabs than fit, the focused one can sit past its right
edge, and the chord then opens a field you cannot see. It still holds the
keyboard, so <kbd>Escape</kbd> leaves it.

**Right-click a tab for its menu**, or press `open-tab-menu` for the focused
one. A right-click focuses the tab first, because every entry acts on the
focused tab:

| Entry | Runs | Shown on |
| --- | --- | --- |
| Rename tab | `rename-tab` | every tab |
| Copy agent name | `copy-agent-name` | a tab whose focused pane has a named Claude in front |
| New shell here | `new-shell-here` | every tab |
| New Claude session here | `new-claude-session-here` | every tab |
| Split right | `split-vertical` | every tab |
| Split down | `split-horizontal` | every tab |
| Close pane | `close-focused` | every tab |

Each entry is the keyboard action of the same name, so it does exactly what
that action's chord does — *Close pane* closes the focused pane of a split and
the whole tab otherwise, asking first as the chord does. <kbd>↑</kbd> and
<kbd>↓</kbd> move the selection, <kbd>Enter</kbd> runs it, <kbd>Escape</kbd> or
a click outside the menu closes it without running anything.

*Copy agent name* is listed when the action would find a name at the moment
the menu opens: a Claude runs in front of the focused pane and has written its
session file, whether the pane was opened as a Claude session or as a shell.
It never appears on Windows, where no Claude is ever named. The menu belongs
to the pane it opened over: if that pane closes or loses focus, the menu goes
with it.

The menu opens in
the middle of the window rather than beside the tab, and screen readers do not
see it: it is drawn by termherd, not by the operating system.

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
