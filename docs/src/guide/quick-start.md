# Quick start

Five minutes, from a cold launch to two sessions side by side with an agent
driving them.

## 1. Launch it

Open the app you installed:

| Platform | Launch |
| --- | --- |
| **macOS** | **TermHerd** in Applications — Launchpad, Spotlight, or double-click it in Finder. |
| **Windows** | **TermHerd** in the Start menu, or its desktop shortcut. |
| **Linux** | **TermHerd** in your application menu (`.deb`), or run the `.AppImage` directly. |

Prefer the terminal? The bare command-line binary is on your `PATH` as
`termherd`, and from a clone it is one command:

```bash
termherd                      # the installed bare binary
cargo run -p termherd-app     # from a clone
```

If you have not installed it yet, see [Installation](./installation.md).

The window opens on the workspace: the **sidebar** on the left, listing every
Claude session TermHerd found by walking `~/.claude/projects`, grouped by
project; the **tab strip** across the top; the focused **terminal** filling the
rest.

Nothing is scanned from your source trees and nothing under `~/.claude` is
written. A first run on a large history takes a moment to walk the tree — the
scan is in-memory (a SQLite cache is roadmapped, not shipped).

## About the chords below

Every shortcut is rebindable, and the defaults differ by platform. The tables
give both columns; the full vocabulary is in
[Keyboard shortcuts](../reference/keyboard.md).

## 2. Open a session

Click a project in the sidebar to expand it, then click a session to open it in
a tab. TermHerd resumes it through the Claude CLI.

From the keyboard, with a terminal focused:

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| New **shell** in the focused session's directory | <kbd>Cmd</kbd>+<kbd>T</kbd> | <kbd>Ctrl</kbd>+<kbd>T</kbd> |
| New **Claude** session in that directory | <kbd>Cmd</kbd>+<kbd>Alt</kbd>+<kbd>T</kbd> | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>T</kbd> |
| Focus the sidebar search box | <kbd>Cmd</kbd>+<kbd>F</kbd> | <kbd>Ctrl</kbd>+<kbd>F</kbd> |

Search matches session titles *and* indexed transcript content; a content hit
shows the matched line under the row so you can tell *why* it matched. See
[The sidebar](../workspace/sidebar.md).

## 3. Arrange it

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Split **vertically** (side by side) | <kbd>Cmd</kbd>+<kbd>D</kbd> | <kbd>Ctrl</kbd>+<kbd>D</kbd> |
| Split **horizontally** (stacked) | <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>D</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>D</kbd> |
| Move focus between panes | <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>←↑↓→</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>←↑↓→</kbd> |
| Next / previous tab | <kbd>Ctrl</kbd>+<kbd>Tab</kbd> / <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd> | same |
| Jump straight to tab 1–9 | <kbd>Cmd</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | <kbd>Ctrl</kbd>+<kbd>1</kbd>…<kbd>9</kbd> |
| Close the focused pane (a lone pane closes its tab) | <kbd>Cmd</kbd>+<kbd>W</kbd> | <kbd>Ctrl</kbd>+<kbd>W</kbd> |
| Reopen the tab you just closed | <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> |

Tabs also reorder by drag-and-drop. Splits resize by keyboard focus only for
now — drag-resize is the remaining piece of `F-terminal-split`.

## 4. Watch what needs you

Each tab carries an activity dot: **busy** while a session is working,
**attention** when it is waiting on you (a permission prompt, a question),
**idle** when it is done. That is also what arms the close confirmation —
closing a tab whose session is mid-command asks first, closing an idle one does
not. [Status and attention](../workspace/status.md).

## 5. Let a session drive the workspace

A Claude session **launched from TermHerd** already has the MCP control surface
wired in. Ask it, inside that session:

> Split this pane, run `cargo test` in the new one, wait for it to finish, and
> tell me what failed.

It will call `split_pane` → `run_in_session` → `wait_for_status` →
`read_terminal`. That act → wait → observe loop, and everything else the
session can reach, is in [Driving termherd over MCP](../mcp/index.md).

## 6. Make it yours

There is no settings panel yet: edit `~/.termherd/settings.json`
(`%USERPROFILE%\.termherd\settings.json` on Windows) and restart. Shell, theme,
terminal colours and font size, close-confirmation policy, sidebar density, GIF
recording budget, the editor a clicked file path opens in, and every key
binding live there — [full reference](../reference/settings.md).

## Where to go next

- [The sidebar](../workspace/sidebar.md) — browsing, search, stars, plans
  and memory
- [The terminal](../workspace/terminal.md) — selection, clickable links and
  paths, scrollback, zoom
- [Capture and record](../workspace/capture.md) — hand an AI assistant the
  app's exact state
