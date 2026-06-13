# Roadmap

Source of truth: [`docs/PRD.md`](docs/PRD.md) §5 (MoSCoW). This file is the
short, scannable view; commits land features here when they ship.

## v0 — M0–M3 (daily-driver)

### Must

- [ ] `F-foundations` — workspace, core, CI, single-instance, tracing
- [x] `F-app-shell` — window, lifecycle, bounds (menu: deferred to M3 with
  the keymap — no native menu API in iced; menu items mirror keymap actions)
- [x] `F-session-browser` — scan + derive + group + list + live fs-watch
  updates (debounced `notify`, FR2)
- [ ] `F-builtin-terminal` — PTY + native terminal widget *(M2: `termherd-pty`
  adapter (`portable-pty` + `alacritty_terminal`, actor-per-session,
  cursor-report reply for ConPTY); iced `canvas` renders the colour grid +
  cursor; raw keyboard routed to the focused PTY; `claude --resume` on a
  session click; PTY resize follows the window. Verified end-to-end on Windows
  resuming a real Claude session. Pending: scrollback + selection)
- [x] `F-search` — in-memory search over digests (was `F-fts-search`;
  the SQLite FTS5 version moved to Should as `F-store-cache`, PRD rev. 4)
  — case-insensitive, titles-only toggle (FR3)
- [ ] `F-status-notifications` — busy / waiting / permission from OSC
  *(M2 in progress: the `pty` reader decodes the raw byte stream with
  `termherd_claude::osc` and emits busy/idle status; the shell shows a badge
  on the focused terminal. Pending: notification/permission distinction,
  sidebar + tab badges, bell/attention)*
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
- [ ] `F-store-cache` — SQLite (WAL) digest cache + FTS5 index
  (lowest Should priority; an optimisation over the in-memory scan/search)

### Could

- [ ] `F-activity-stats`
- [ ] `F-session-grid` — a layout preset over the pane model
- [ ] `F-scheduled-tasks`

### Unsure (deferred)

- [ ] `F-file-diff-panel`
- [ ] `F-mcp-ide-bridge`
