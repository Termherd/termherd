# Roadmap

Source of truth: [`docs/PRD.md`](docs/PRD.md) §5 (MoSCoW). This file is the
short, scannable view; commits land features here when they ship.

## v0 — M0–M3 (daily-driver)

### Must

- [ ] `F-foundations` — workspace, core, CI, store, single-instance, tracing
- [x] `F-app-shell` — window, lifecycle, bounds (menu: deferred to M3 with
  the keymap — no native menu API in iced; menu items mirror keymap actions)
- [ ] `F-session-browser` — scan + derive + group + list
- [ ] `F-builtin-terminal` — PTY + native terminal widget
- [ ] `F-fts-search` — SQLite FTS5 over content
- [ ] `F-status-notifications` — busy / waiting / permission from OSC
- [ ] `F-settings` (thin) — shell select, theme, window prefs
- [ ] `F-packaging-ci` — signed mac/win/linux builds + CI gate (3-OS matrix)
  — *pipeline + CI gate in place; "signed" pending certificates (OQ5)*
- [ ] `F-session-tabs` — tabbed open sessions
- [ ] `F-terminal-split` — split panes (h/v), focus, resize
- [ ] `F-keyboard-shortcuts` — configurable keymap → actions

### Should (post v0)

- [ ] `F-fork-detection`
- [ ] `F-session-metadata`
- [ ] `F-jsonl-viewer`
- [ ] `F-plans-memory`
- [ ] `F-auto-update`

### Could

- [ ] `F-activity-stats`
- [ ] `F-session-grid` — a layout preset over the pane model
- [ ] `F-scheduled-tasks`

### Unsure (deferred)

- [ ] `F-file-diff-panel`
- [ ] `F-mcp-ide-bridge`
