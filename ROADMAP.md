# Roadmap

Source of truth: [`docs/PRD.md`](docs/PRD.md) §5 (MoSCoW). This file is the
short, scannable view; commits land features here when they ship.

## v0 — M0–M3 (daily-driver)

### Must

- [ ] `F-foundations` — workspace, core, CI, single-instance, tracing
- [x] `F-app-shell` — window, lifecycle, bounds (menu: deferred to M3 with
  the keymap — no native menu API in iced; menu items mirror keymap actions)
- [ ] `F-session-browser` — scan + derive + group + list — *scan adapter,
  grouping and sidebar shipped; missing: live fs-watch updates (FR2)*
- [ ] `F-builtin-terminal` — PTY + native terminal widget
- [ ] `F-search` — in-memory search over digests (was `F-fts-search`;
  the SQLite FTS5 version moved to Should as `F-store-cache`, PRD rev. 4)
- [ ] `F-status-notifications` — busy / waiting / permission from OSC
- [ ] `F-settings` (thin) — shell select, theme, window prefs
- [ ] `F-packaging-ci` — signed mac/win/linux builds + CI gate (3-OS matrix)
  — *pipeline + CI gate in place; "signed" pending certificates (OQ5)*
- [ ] `F-session-tabs` — tabbed open sessions
- [ ] `F-terminal-split` — split panes (h/v), focus, resize
- [ ] `F-keyboard-shortcuts` — configurable keymap → actions

### Should (post v0)

- [ ] `F-store-cache` — SQLite (WAL) digest cache + FTS5 index
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
