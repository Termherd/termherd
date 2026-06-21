# Roadmap

Source of truth: [`docs/PRD.md`](docs/PRD.md) §5 (MoSCoW). This file is the
short, scannable view; commits land features here when they ship.

## Working order (next up)

Execution priority across the open Musts and the feedback issues (#18–#29,
gist `d1d02e5`).
The MoSCoW buckets below stay tied to PRD §5; this block is just the order to
pick work in. GH `P0`/`P1`/`P2` labels mirror it.

1. **Finish the Musts** (`v0.1.0` milestone) — release packaging is now: macOS
   via **Homebrew** (#61, a cask installs the unsigned bundle without a
   Gatekeeper block), **Linux** signed checksums (#52), and the
   `F-plans-memory` editing slice (#53). Deferred to P2, post-release: real
   macOS Developer ID signing (#51 — no free OSS path, $99/yr) and **Windows**
   Authenticode via the free **SignPath Foundation** (#62 — viable, not parked;
   gated only on a policy page + MFA + their approval wait). See feature-torture
   `F-packaging-ci.md`.
2. **P0 — bugs in shipped features:** #19 (Tab key not forwarded — core
   terminal path), #18 (tab status stuck — misleading status badge).
3. **P1 — cheap, on-thesis wins:** #26 (Ctrl+Number tabs), #23 (sidebar launch
   affordances: `$`/🤖 buttons + collapse-on-name; adds the missing fresh-Claude
   launch mode — FR4a). (#21 hide pane / Ctrl+B and #20 archive confirm —
   shipped.)
4. **P2 — polish:** #25 drag-reorder tabs (FR5), #27 cursor +
   double-click. (#24 tab titles, #28 link click, #29 OS
   notifications — shipped.)
5. **Design-first backlog** — see below; don't code until scoped.

`F-terminal-split` (Should) isn't in the feedback but its core already landed —
the cheapest large/visible feature left on the board if a release needs a
headline. The UI slice is now scoped as #54 (fixed-ratio split + focus +
per-pane geometry) with drag-resize split out to #55 (feature-torture
`F-terminal-split.md`).

> **Feature-torture pass (2026-06-20).** The seven open/backlog features were
> each pressure-tested; reports live in `.personal/feature-torture/reports/`.
> Verdicts graduated nine slices into issues #51–#60 and the `v0.1.0`
> milestone; the residual design-first items are marked below.

## v0 — M0–M3 (daily-driver)

### Must

- [x] `F-foundations` — workspace, core, CI, single-instance, tracing
- [x] `F-app-shell` — window, lifecycle, bounds (menu: deferred to M3 with
  the keymap — no native menu API in iced; menu items mirror keymap actions)
- [x] `F-session-browser` — scan + derive + group + list + live fs-watch
  updates (debounced `notify`, FR2); a per-project disclosure triangle folds
  its session list, persisted to `~/.termherd/collapsed.json` (#22)
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
  certificates (OQ5). **Split by platform** (feature-torture 🧬). macOS: the
  Homebrew path (#61) is **parked P2** — Homebrew 5.1 removed
  `--no-quarantine` (all taps), so an unsigned cask can't bypass Gatekeeper
  and casks failing it are unsupported after 2026-09-01; v0.1.0 therefore
  ships macOS **unsigned** (`.dmg` + manual `xattr`), and Developer ID
  notarization (#51, no free OSS path) is now the sole fluent macOS path,
  deferred to GitHub traction / a sponsor ($99/yr). Linux ships **signed
  checksums** (#52). **Windows** Authenticode via free **SignPath
  Foundation** (#62, P2)*
- [x] `F-session-tabs` — tabbed open sessions (M3): every launched session is
  a tab; a tab strip switches between them, each chip carrying its activity
  dot (the FR8 tab badge) and a close button that kills the session's PTY —
  the first UI-driven `Effect::Kill`. Tab tree edits (`activate`/`close_tab`,
  most-urgent `tab_status`) are pure in `core`. Tab labels follow the title
  Claude reports over OSC 0 (#24): the `osc` decoder now carries the title
  text, the `pty` reader forwards a change as `PtyEvent::Title`, and
  `Workspace::set_session_title` relabels the hosting tab. Drag-reorder (FR5) and
  keyboard switching (deferred to `F-keyboard-shortcuts`) still to come
- [x] `F-keyboard-shortcuts` — configurable keymap → actions (M3): pure
  `KeyChord -> Action` map in `core::keymap` with a chord-string parser and
  platform-aware defaults; the `keys` section of `settings.json` overrides any
  action. Drives copy/paste, `Ctrl+Tab`/`Ctrl+Shift+Tab` cycling, close-tab,
  focus-search, `toggle-sidebar` (hide pane / Ctrl+Cmd+B, #21) and
  `scroll-top`/`scroll-bottom` (Ctrl/Cmd+Up/Down, #44) today;
  `split-*` / `focus-next/prev` bind as those features land
- [x] `F-session-metadata` — star / rename / archive / custom titles for
  sessions (M3, moved to Must in PRD rev. 6): a `SessionMeta` overlay in
  `core` persisted to `~/.termherd/metadata.json` (never touching `~/.claude`);
  the browser pins starred sessions, hides archived behind a toggle, and shows
  custom titles. Star / archive / inline rename (✎ → edit field) are all
  sidebar controls
- [x] `F-plans-memory` — browse/edit plans + `CLAUDE.md` (M3, moved to Must in
  PRD rev. 6): a sidebar "Plans & mémoire" section lists `~/.claude/plans/*.md`,
  the global `CLAUDE.md` and each project's `CLAUDE.md`, opening one in the main
  pane (off-thread read via the `docs` adapter). The editing slice (#53) added
  in-app editing with a narrow, ADR-ratified write-scope
  ([`docs/adr/0001`](docs/adr/0001-plans-memory-write-scope.md)): writes reach
  only `~/.claude/CLAUDE.md`, `~/.claude/plans/*.md` and project `CLAUDE.md`,
  guarded by a pure `core::docscope` predicate, an mtime concurrency check, and
  an atomic temp-then-rename save

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
  recursive pane rendering, click-to-focus, and per-pane PTY geometry — #54
  (MVP: fixed-ratio split; `core::Workspace` stays the single source of truth)
  with drag-resize as fast-follow #55. Note: the keymap actions are currently
  dropped at `shell.rs:721` (`=> Task::none()`) — wiring them is step one
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
- [ ] `F-mcp-control-surface` — termherd *exposes* an MCP server over its own
  control/config surface (`list_options`/`set_option` + schema resource, plus
  orchestration tools: open session, split pane, focus, rename tab, run in
  another session), driven by the in-app Claude sessions. Inverse of
  `F-mcp-ide-bridge` (termherd is the server, the session is the client).
  Filed as #90; still design-first — needs a `/feature-torture` pass to settle
  scope before slices become standalone issues

### Backlog — needs definition (from feedback gist, 2026-06-17)

Routed here (not to GH issues) because each needs design before it can be
scoped. Source: feedback gist `d1d02e5`.
The well-defined items from the same gist are tracked as issues #18–#29.

- [ ] `F-favorites` — favorites in the sidebar. **Designed (🧬 split,
  feature-torture `F-favorites.md`)**: "star" == "favorite" is one concept.
  Graduated to #56 (cross-project Favorites section, reusing the shipped
  session star) and #57 (repo-level favoriting, a `project_path`-keyed overlay
  in `~/.termherd/metadata.json`, never `~/.claude`)
- [ ] `F-search-ux` — search activation + scope. **Designed (✂️ reshape,
  feature-torture `F-search-ux.md`)**: most of it already shipped — `Cmd+F`
  focuses search and `filter_projects` already searches content + titles across
  every project. The one real gap, a **match snippet**, graduated to #58. A
  flat relevance-ranked results view is explicitly *not* pursued (the grouped
  in-context filter is better UX)
- [ ] `F-keymap-advanced` — keymap concerns from the gist that need design,
  layered on the shipped `F-keyboard-shortcuts`:
  - ~~localized number-row handling (AZERTY: `&`→1, `é`→2, …) so Ctrl/Cmd+Number
    (issue #26) works on non-QWERTY layouts~~ — **done** with #26: the number
    row is matched by physical key position, so Ctrl/Cmd+1…9 land on the same
    keys on every layout (QWERTY/AZERTY/QWERTZ/…)
  - per-command keymap configuration (different bindings per running command)
    — **stays design-first** (feature-torture 🧬 `F-keymap-advanced.md`):
    blocked on foreground-process detection (macOS `tcgetpgrp` vs Windows
    ConPTY gap); file only once that's designed
  - a configurable "bypass" key so a modifier passes through to the terminal
    instead of the app (cf. Ghostty `macos-option-as-alt`) — **graduated to
    #59** (the cheap, high-value slice)
- [ ] `F-i18n` — internationalization. **Parked** (feature-torture ⏸
  `F-i18n.md`): heaviest, least urgent. The pressure test surfaced a *present*
  issue though — the UI was hardcoded **French** in an English-README repo, with
  no string externalization. Canonical UI language settled as **English-first**;
  the externalization precursor shipped (#60), centralising every user-facing
  string in `crates/app/src/strings.rs`. Locale machinery (catalogues,
  selection, width/RTL) stays parked until a non-English user base appears

### Unsure (deferred)

- [ ] `F-file-diff-panel`
