# The settings panel

The **⚙** at the top of the sidebar — or
<kbd>Cmd</kbd>/<kbd>Ctrl</kbd>+<kbd>,</kbd> (rebindable as `open-settings`) —
opens a panel over the workspace. <kbd>Escape</kbd>, **Close**, or a click
outside it closes it.

```text
┌ Settings ─────────────────────────── Close ┐
│ Interface theme        [ Solarized Light ▾ ] │
│ Terminal colours       [ Solarized Light ▾ ] │
└──────────────────────────────────────────────┘
```

Today it holds the appearance. The rest of `settings.json` is still edited in
the file — see [settings.json](../reference/settings.md).

## A pick applies at once, and saves itself

There is no Apply button. Picking a value:

- **Interface theme** restyles the sidebar, tab strip and buttons on the next
  frame. `Dark`, `Light`, and four presets named after the terminal schemes.
- **Terminal colours** recolours every open terminal, not only new ones.
  `Built-in` is the default palette. Explicit colour overrides in
  `terminal.colors` still win over the scheme, as they do at startup.

Each pick writes its own key to `settings.json` — `theme` or
`terminal.colors.scheme` — and leaves the rest of the file as you wrote it.
A file that does not parse is left alone rather than rewritten.

Pair a preset with the scheme of the same name for a chrome that matches the
grid it frames.

## One thing a live change cannot reach

A program asks the terminal for its background colour when it starts, and
picks a light or a dark look from the answer. Claude Code does this. A Claude
session that was already running keeps the look it chose until it restarts;
TermHerd's own colours change at once. New sessions get it right.
