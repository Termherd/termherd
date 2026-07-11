# Roadmap

Source of truth: [`docs/PRD.md`](docs/PRD.md) §5 (MoSCoW). This file is the
short, scannable view; commits land features here when they ship.

## Working order (next up)

Execution priority across the open Musts and the feedback issues (#18–#29,
gist `d1d02e5`).
The MoSCoW buckets below stay tied to PRD §5; this block is just the order to
pick work in. GH `P0`/`P1`/`P2` labels mirror it.

> **Reprioritization (2026-07-05).** The board had flattened at P2; a fresh
> pass differentiated it. Current **P1**: #102 (scroll-drift correctness bug),
> #54 (split-pane UI — the release headline), #79 + #80 (confirm-on-running-
> process guards), #119 (reflect Claude's own `/rename` in the tab), plus the
> `F-quality-gates` slices #105/#106/#107. All three code-signing paths dropped
> to **P3**. #90 stays P2 but is marked `needs-design`; #55 is blocked-by #54.
> #110 closed — rung 2 shipped as #124/#126.

1. **Finish the Musts** (`v0.1.0` milestone) — macOS ships **unsigned**
   (`.dmg` + manual `xattr`); **Linux** signed checksums (#52, done) and the
   `F-plans-memory` editing slice (#53, done) are in. All code signing is now
   **P3**, deferred until GitHub traction / a sponsor: macOS Developer ID
   notarization (#51 — no free OSS path, $99/yr), the **Homebrew** cask (#61 —
   parked since Homebrew 5.1 removed `--no-quarantine`, so an unsigned cask
   can't bypass Gatekeeper), and **Windows** Authenticode via **SignPath
   Foundation** (#62 — viable, but not release-blocking). What's left in-bucket
   is `F-quality-gates` (#105/#106/#107, P1). See feature-torture
   `F-packaging-ci.md`.
2. **P1 — correctness + the headline feature:** #102 (scroll-drift property
   failure — the one open correctness bug), #54 (fixed-ratio split-pane UI —
   cheapest large/visible feature, core already landed), #79/#80 (don't kill a
   running Claude by accident), #119 (live tab name), and `F-quality-gates`
   #105/#106/#107.
3. **P2 — polish:** #36 (copy-on-select), #55 (drag-resize, **blocked-by #54**),
   #56/#57 (favorites), #59 (modifier bypass), #37 (settings template), #84
   (OSC 8 links), #85 (inline images), #86 (bg notifications), #82 (link-cursor
   bug), #90 (MCP control surface, **`needs-design`**), #114 (Cmd+M minimize).
4. **P3 — parked / not actionable now:** code signing — #51, #61, #62.
5. **Design-first backlog** — see below; don't code until scoped.

`F-terminal-split` (Should) isn't in the feedback but its core already landed —
the cheapest large/visible feature left on the board and the release headline,
now **P1**. The UI slice is scoped as #54 (fixed-ratio split + focus + per-pane
geometry) with drag-resize split out to #55 (blocked-by #54; feature-torture
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
  its session list, persisted to `~/.termherd/collapsed.json` (#22); long
  groups list only the N most recent sessions with a "… N more" expander
  (`sidebar.session_limit` in `settings.json`, default 5, 0 = all; #131)
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
  `F-session-tabs`); the bell is decoded but not treated as activity. #86:
  `core` now tracks OS window focus (`Event::WindowFocusChanged`) so a
  background tab's notification still reaches the OS while termherd itself
  is focused — only the active tab's focused pane skips the banner
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
  Homebrew path (#61) is **parked P3** — Homebrew 5.1 removed
  `--no-quarantine` (all taps), so an unsigned cask can't bypass Gatekeeper
  and casks failing it are unsupported after 2026-09-01; v0.1.0 therefore
  ships macOS **unsigned** (`.dmg` + manual `xattr`), and Developer ID
  notarization (#51, no free OSS path, now **P3**) is the sole fluent macOS
  path, deferred to GitHub traction / a sponsor ($99/yr). Linux ships **signed
  checksums** (#52, done) — a `sign-release.yml` workflow attaches a
  `SHA256SUMS` over the Linux tarballs and a sigstore *keyless*
  (GitHub OIDC, no stored key) build-provenance attestation; verify with
  `gh attestation verify <artifact> --repo Termherd/termherd`. **Windows**
  Authenticode via free **SignPath
  Foundation** (#62, now **P3** — viable, but not release-blocking)*
- [ ] `F-quality-gates` — intrinsic-quality CI gates beyond the existing
  fmt/clippy/test/deny set, targeting the structural/maintainability axis
  (complexity, domain boundaries, merge-conflict risk). Scoped from a
  brainstorm (`brainstorm/20260627-ci-quality-gates.md`). **P1:** function
  length (#105), unused deps (#106), and the crate-level dependency rule as an
  architecture fitness function enforcing the hexagonal inward-only invariant
  (#107). **P2 follow-ups** (not yet filed): intra-crate module rules (D
  phase 2, `cargo-modules`/archtest) and cognitive-complexity (signal C).
  **P3 / report-only** (blocked on a quality-report home): file length
  (signal A) and churn×size hotspots (signal J). Dropped: MSRV check,
  `todo!`→deny, PR-size warning (rationale in the report). `tech-health`
- [x] `F-session-tabs` — tabbed open sessions (M3): every launched session is
  a tab; a tab strip switches between them, each chip carrying its activity
  dot (the FR8 tab badge) and a close button that kills the session's PTY —
  the first UI-driven `Effect::Kill`. Tab tree edits (`activate`/`close_tab`,
  most-urgent `tab_status`) are pure in `core`. Tab labels: a resumed tab takes
  the session name from the scanned digest (#109/#118) — current Claude (2.1.195)
  renders status in-band in its TUI and emits no OSC title, so the OSC-0 override
  (#24) never fires there; a fresh/unscanned session keeps the `<repo>` kind
  label. The OSC plumbing stays in place and still wins where a Claude does emit
  a title: the `osc` decoder carries the title text, the `pty` reader forwards a
  change as `PtyEvent::Title`, and `Workspace::set_session_title` relabels the
  hosting tab — which also lets a sidebar rename retitle the open tab live.
  Reflecting Claude's *own* `/rename` and live task name is tracked as #119.
  Hovering a tab shows
  the session's fuller description — the same hover card the sidebar uses for a
  resumed session, a title + cwd card otherwise (#76, `App::tab_record` resolves
  the record so the two surfaces stay single-sourced). Drag-reorder (FR5) and
  keyboard switching (deferred to `F-keyboard-shortcuts`) still to come
- [x] `F-close-confirm-policy` — configurable close confirmation for tab close
  and app quit (`close.tab` / `close.app` in `settings.json`, each
  `alwaysConfirm` / `confirmWhenActive` / `noConfirmation`). One pure decision
  seam (`ConfirmClose::confirms(active)`) backs both paths; `confirmWhenActive`
  reuses #140's `has_running_process` predicate — a tab keys off
  `App::tab_has_running_process`, a quit off the app-wide `any_running_process`
  (both over `LiveSession::has_running_process`), so an idle shell closes/quits
  silently while a working shell or live Claude confirms. Both default to
  `confirmWhenActive`, preserving #140's shipped tab behaviour and extending the
  same predicate to quit — which is #80 (an all-idle app now quits without a
  prompt). Built on #79/#140 (closed); the config surface is the new part.
  Known gap: the predicate reads a plain shell running a non-Claude foreground
  program (vim, a long `make`) as idle, so it can be closed/quit silently —
  better foreground-process detection is tracked in #143
- [x] `F-keyboard-shortcuts` — configurable keymap → actions (M3): pure
  `KeyChord -> Action` map in `core::keymap` with a chord-string parser and
  platform-aware defaults; the `keys` section of `settings.json` overrides any
  action. Drives copy/paste, `Ctrl+Tab`/`Ctrl+Shift+Tab` cycling, close-tab,
  focus-search, `toggle-sidebar` (hide pane / Ctrl+Cmd+B, #21),
  `scroll-top`/`scroll-bottom` (Ctrl/Cmd+Up/Down, #44), and the in-context tab
  shortcuts `new-shell-here` / `new-claude-session-here` (Ctrl/Cmd+T,
  Ctrl/Cmd+Alt+T, #77) + `reopen-closed-tab` (Ctrl/Cmd+Shift+T, #78, a LIFO
  closed-tab stack in `core`) today; `split-*` / `focus-next/prev` bind as those
  features land
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
  Claude reintroduces forked session files. A neighbouring but distinct case
  *does* occur: Claude carries a `customTitle` across `/clear` into a fresh,
  unrelated session, so two real files read alike — handled not by fork
  detection but by the summary disambiguator (#93), not a fork
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
- [ ] `F-terminal-images` — render images inline in the terminal (iTerm2 OSC
  1337 / Sixel / Kitty graphics), sibling to `F-jsonl-viewer` /
  `F-file-diff-panel` in the rendering family. Filed as #85. **Parked**
  (feature-torture ⏸ `F-terminal-images.md`): the issue's stated symptom
  ("garbage escape text") doesn't reproduce — `vte`/`alacritty_terminal`
  already discards unrecognised OSC/DCS/APC sequences cleanly; the real gap
  is silence, not garbage. No slice is cheap: even a placeholder-only render
  needs the same chunked-payload reassembly `crates/claude/src/osc.rs`
  explicitly punts on today, across 3 mutually incompatible protocols (OSC/
  DCS/APC). Zero demand signal beyond the filed issue. Revisit on a real
  user report of the silent drop, or a free cycle after `F-terminal-split`
  (#54/#55)
- [ ] `F-auto-update`
- [ ] `F-store-cache` — SQLite (WAL) digest cache + FTS5 index
  (lowest Should priority; an optimisation over the in-memory scan/search)

### Could

- [ ] `F-activity-stats`
- [ ] `F-launch-profiles` — parameterised session launch. **Tortured (✂️
  reshape, feature-torture `F-launch-profiles.md`).** The written framing
  (arbitrary flags: `--add-dir`, `--model`, `--mcp-config`, launch profiles)
  mostly duplicates in-session slash commands (`/add-dir`, `/model`) and what
  `--resume` restores. The one non-redundant slice: **persistent per-project
  `--add-dir`, applied to both fresh and `--resume` launches** — a multi-root
  repo opens already reading its sibling dir, set once on the repo (flag
  composition `claude --resume {id} --add-dir X` verified; `--add-dir` is
  variadic). Store it in the `project_path`-keyed
  `~/.termherd/metadata.json` overlay (reuse #57), not a new settings schema;
  ride the `Launch`-enum edit on `F-antigravity-sessions` (#162) to touch it
  once. **Unblocked: #57's `repos` overlay shipped**, so `RepoMeta` can now grow
  an `extra_dirs` field. Today `Launch::Claude` carries only `{ resume }`
  (`crates/core/src/app.rs`) and the command is *typed* into the shell
  (`launch_command`, `crates/pty/src/lib.rs`), so the one real cost is
  cross-shell path quoting (pwsh vs bash)
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
  the full scope before the remaining slices become standalone issues. A first,
  limited slice has landed: `crates/mcp` (`termherd-mcp`), a **read-only** stdio
  MCP server exposing `list_options` + the option schema resource over the
  existing `settings.json`, with the protocol/option logic pure and unit-tested.
  `set_option` (writes), the `keys` surface and the orchestration tools
  (open session / split / focus / rename / run-in-session) are still to come

- [ ] `F-capture` — capture termherd (screenshots / screencasts) along a
  fidelity ladder, for three goals: **G1** dev/AI debug loop, **G2** promo &
  tutorial visuals, **G3** bug-repro recordings (devs now, maybe end users
  later). Brainstorm: `brainstorm/20260627-auto-capture-screenshots.md`.
  Grounding: termherd is an iced 0.14 GUI, so it ships
  `window::screenshot()` (cross-platform, `png` already a dep) and
  `iced_test::screenshot()` for headless CI; TTY recorders (asciinema/VHS)
  only capture the inner terminal, not the GUI shell. Capture is an
  `Event`→`Effect` (pure `core`, I/O in `app`), surviving the hexagonal
  tightening. Ladder:
  - **Rung 0+1 (G1) — shipped (#108)** (`tech-health`): ⌘⇧S → `Event::Capture`
    → `Effect::Capture` → a JSON state+PTY-text dump *and* an iced PNG to
    `~/.termherd/captures/capture-<ts>.{json,png}` an AI reads by newest stamp.
    The cheap, on-thesis first slice.
  - **Rung 2 (G3) — shipped (#124, #126)** (`tech-health`): reshaped ✂️ by
    feature-torture (`.personal/feature-torture/reports/F-capture-rung2.md`)
    to **one dev-only GIF screencast** slice (⌘⇧R toggle, pure-Rust `gif`,
    screenshot-loop driven by the window's present clock (`window::frames()`,
    throttled to fps — #128, fixing the idle-window time-lapse), hard frame cap;
    record state machine pure in `core`, encoder on a dedicated thread in `app`).
    **In-app mp4 was cut** —
    `x264` is GPL (relicenses the MIT binary) and `openh264` compiles C via
    `build.rs` on all 3 CI legs, both breaking the no-FFI / MIT / no-`unsafe`
    posture; **G2 promo polish routes to external recorders**. Settings-
    configurable budget (fps/cap/scale) is a follow-up (#127).
  - **Seeded demo-data mode — design-first:** fixtures of fake sessions for
    clean, reproducible captures. Force-multiplier for G2/G3, not a capture
    method; revisit when rung 2 comes forward.

### Backlog — needs definition (from feedback gist, 2026-06-17)

Routed here (not to GH issues) because each needs design before it can be
scoped. Source: feedback gist `d1d02e5`.
The well-defined items from the same gist are tracked as issues #18–#29.

- [ ] `F-favorites` — favorites in the sidebar. **Designed (🧬 split,
  feature-torture `F-favorites.md`)**: "star" == "favorite" is one concept.
  Graduated to #56 (cross-project Favorites section, reusing the shipped
  session star) and #57 (repo-level favoriting, a `project_path`-keyed overlay
  in `~/.termherd/metadata.json`, never `~/.claude`). Both children
  implemented: **#57** — a `repos` map in the overlay (`RepoMeta`), a star on
  each project header that pins the group to the top, and a flat→wrapped JSON
  migration; **#56** — a cross-project "★ Favorites" section at the top of the
  sidebar aggregating every starred session (coexists with the in-group pin —
  the favourite is a shortcut, not a move). Epic ticks once both PRs merge and
  the issues close
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
