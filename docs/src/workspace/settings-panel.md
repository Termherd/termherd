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

Today it holds the appearance. The rest of `settings.json` is edited in the
file, which applies on save too — see [settings.json](../reference/settings.md).
An edit to the file while the panel is open shows in it at once.

## A pick applies at once, and saves itself

There is no Apply button. Picking a value:

- **Interface theme** restyles the sidebar, tab strip and buttons on the next
  frame. Every theme iced ships: 22. While the terminal colours are
  `Same as theme`, it recolours every open terminal with them.
- **Terminal colours** recolours every open terminal, not only new ones.
  `Same as theme`, the default, follows the interface theme; a scheme picked
  here stays when the theme changes. Explicit colour overrides in
  `terminal.colors` still win over the scheme, as they do at startup.

Each pick writes its own key to `settings.json` — `theme` or
`terminal.colors.scheme` — and leaves the rest of the file as you wrote it.
A file that does not parse is left alone rather than rewritten.

With `Same as theme`, the window matches by itself. 18 themes have a scheme
of their own; [the list](../reference/settings.md#theme) shows which, and
what the others bring.

## One thing a live change cannot reach

A program asks the terminal for its background colour when it starts, and
picks a light or a dark look from the answer. Claude Code does this. A Claude
session that was already running keeps the look it chose until it restarts;
TermHerd's own colours change at once. New sessions get it right.
