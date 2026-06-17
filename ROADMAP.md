# Roadmap

Source of truth: [`docs/PRD.md`](docs/PRD.md) §5 (MoSCoW). This file is the
short, scannable view; commits land features here when they ship.

## v0 — M0–M3 (daily-driver)

### Must

- [x] `F-foundations` — workspace, core, CI, single-instance, tracing
- [x] `F-app-shell` — window, lifecycle, bounds (menu: deferred to M3 with
  the keymap — no native menu API in iced; menu items mirror keymap actions)
- [x] `F-session-browser` — scan + derive + group + list + live fs-watch
  updates (debounced `notify`, FR2)
- [x] `F-builtin-terminal` — PTY + native terminal widget (M2):
  `termherd-pty` adapter (`portable-pty` + `alacritty_terminal`,
  reader + terminal thread per session, cursor-report reply for ConPTY);
  iced `canvas` renders the colour grid + cursor; raw keyboard routed to the
  focused PTY; wheel scrollback; drag-to-select + copy; `claude --resume` on
  a session click; PTY resize follows the window. Verified end-to-end on
  Windows resuming a real Claude session.
- [x] `F-search` — in-memory search over digests (was `F-fts-search`;
  the SQLite FTS5 version moved to Should as `F-store-cache`, PRD rev. 4)
  — case-insensitive, titles-only toggle (FR3)
- [x] `F-status-notifications` — busy / waiting / permission from OSC (M2):
  the `pty` reader decodes the raw byte stream with `termherd_claude::osc`;
  busy/idle titles plus an OSC 9 notification → a distinct `Attention` status
  (sticky over idle, cleared by work). Surfaced as a badge on the focused
  terminal, a per-session dot in the sidebar, and a dot on each tab (with
  `F-session-tabs`); the bell is decoded but not treated as activity
- [x] `F-settings` (thin) — shell select, theme, window prefs (M3):
  `~/.termherd/settings.json` (serde, defaults on missing/corrupt) carries a
  shell profile (program + args), injected into the `PtyManager` so each
  session launches the chosen shell, and a GUI theme (dark/light) wired to the
  iced chrome. Window bounds keep their own `window.json` (FR12). File-based
  for now; an in-app settings panel is the full version later
- [ ] `F-packaging-ci` — signed mac/win/linux builds + CI gate (3-OS matrix)
  — *bare-binary pipeline (cargo-dist: curl|sh / PowerShell installers) plus
  the CI gate are in place; desktop installers now build too — a
  `cargo-packager` config (`[package.metadata.packager]` + an app icon set) and
  a `package.yml` workflow produce macOS `.app`/`.dmg`, Windows `.msi`/`.exe`
  and Linux `.deb`/`.AppImage`, attached to the release. macOS `.app`/`.dmg`
  verified locally. Only "signed" remains — bundles are unsigned pending
  certificates (OQ5)*
- [x] `F-session-tabs` — tabbed open sessions (M3): every launched session is
  a tab; a tab strip switches between them, each chip carrying its activity
  dot (the FR8 tab badge) and a close button that kills the session's PTY —
  the first UI-driven `Effect::Kill`. Tab tree edits (`activate`/`close_tab`,
  most-urgent `tab_status`) are pure in `core`. Drag-reorder (FR5) and
  keyboard switching (deferred to `F-keyboard-shortcuts`) still to come
- [x] `F-keyboard-shortcuts` — configurable keymap → actions (M3): pure
  `KeyChord -> Action` map in `core::keymap` with a chord-string parser and
  platform-aware defaults; the `keys` section of `settings.json` overrides any
  action. Drives copy/paste, `Ctrl+Tab`/`Ctrl+Shift+Tab` cycling, close-tab
  and focus-search today; `split-*` / `focus-next/prev` bind as those features
  land
- [x] `F-session-metadata` — star / rename / archive / custom titles for
  sessions (M3, moved to Must in PRD rev. 6): a `SessionMeta` overlay in
  `core` persisted to `~/.termherd/metadata.json` (never touching `~/.claude`);
  the browser pins starred sessions, hides archived behind a toggle, and shows
  custom titles. Star / archive / inline rename (✎ → edit field) are all
  sidebar controls
- [ ] `F-plans-memory` — browse/edit plans + `CLAUDE.md` (M3, moved to Must in
  PRD rev. 6): **read-only browse/view shipped** — a sidebar "Plans & mémoire"
  section lists `~/.claude/plans/*.md`, the global `CLAUDE.md` and each
  project's `CLAUDE.md`, opening one read-only in the main pane (off-thread
  read via the new `docs` adapter). Editing + the `~/.claude` write-scope
  relaxation are the remaining slice

### Should (post v0)

- [ ] `F-fork-detection` — fork / plan-accept detection (**blocked**, PRD
  rev. 7): an investigation of 23 real `~/.claude` sessions found none of the
  signals the original feature relied on — `forkedFrom` is never populated,
  no message `uuid` is shared across sessions, and there are no sub-120s
  session transitions. Current Claude Code appends a resume to the same file
  (stable `sessionId`), so separate fork files don't occur. Revisit only if
  Claude reintroduces forked session files
- [ ] `F-terminal-split` — split panes (h/v), focus, resize (moved from Must,
  PRD rev. 5): the pure pane-tree core already landed — `Workspace::split` /
  `close_focused` / `focus_next`/`prev` and the `App` events
  `SplitFocused`/`CloseFocusedPane`/`FocusNextPane`/`Prev`, all unit-tested,
  plus the `split-*` / `focus-*` keymap actions. What remains is the iced
  recursive pane rendering, click-to-focus, and per-pane PTY geometry
- [ ] `F-jsonl-viewer`
- [ ] `F-auto-update`
- [ ] `F-store-cache` — SQLite (WAL) digest cache + FTS5 index
  (lowest Should priority; an optimisation over the in-memory scan/search)

### Could

- [ ] `F-activity-stats`
- [ ] `F-session-grid` — a layout preset over the pane model
- [ ] `F-scheduled-tasks`
- [ ] `F-mcp-ide-bridge` — live MCP/IDE bridge to Claude (moved from Unsure,
  PRD rev. 6); decoupled from the still-Unsure diff panel

### Unsure (deferred)

- [ ] `F-file-diff-panel`
