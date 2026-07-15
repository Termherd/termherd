# Roadmap

Source of truth: [`docs/PRD.md`](docs/PRD.md) §5 (MoSCoW). This file is the
short, scannable view; commits land features here when they ship.

## Working order (next up)

Execution priority across the open Musts and the feedback issues (#18–#29,
gist `d1d02e5`).
The MoSCoW buckets below stay tied to PRD §5; this block is just the order to
pick work in. GH `P0`/`P1`/`P2` labels mirror it.

> **Reprioritization (2026-07-12).** Supersedes the 2026-07-05 pass. Closed
> since: #79 + #80 (confirm-on-running guards → `F-close-confirm-policy`),
> #56 + #57 (favorites → `F-favorites`), #86 (background notifications), and the
> `F-quality-gates` P1 slices #105/#106/#107, plus the **complete intra-crate
> refactor #167–#173** (clusters A–G — the god-object splits and the CI lock-in
> gate `intra-crate-arch`). Current **P1**: #102 (scroll-drift correctness bug),
> #54 (split-pane UI — the release headline), #119 (reflect Claude's own
> `/rename` in the tab). All three code-signing paths are **P3**. #90
> stays P2 but is marked `needs-design`; #55 is blocked-by #54.

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
   cheapest large/visible feature, core already landed), #119 (live tab name).
   Done since the last pass: #54 (split-pane UI), the **complete intra-crate
   refactor #167–#173** (`tech-health` — the shell/terminal/core/scan/pty
   god-object splits, clusters B/C/A/E/F/D, plus the CI lock-in gate G #173),
   #79/#80 (running-process guards) and the `F-quality-gates` P1 slices
   #105/#106/#107.
3. **P2 — polish:** #36 (copy-on-select), #55 (drag-resize, **blocked-by #54**),
   #59 (modifier bypass), #37 (settings template), #84 (OSC 8 links), #85
   (inline images), #82 (link-cursor bug), #90 (MCP control surface,
   **`needs-design`**), #114 (Cmd+M minimize). Done since the last pass:
   #56/#57 (favorites) and #86 (bg notifications).
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
  (`sidebar.session_limit` in `settings.json`, default 5, 0 = all; #131).
  Section headers fold on a title click, not only the disclosure triangle —
  the Favorites and Plans & mémoire titles gained the parity a project header
  already had, via a shared `section_header` builder (#146). Thin theme-aware
  rules separate the sidebar sections (Favorites / Plans & mémoire / Projects)
  so the grouping reads at a glance (#150). The sidebar view was extracted to
  its own `shell/view/sidebar.rs` with per-section row builders, dropping the
  `too_many_lines` allow (C2 of the intra-crate refactor, #168)
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
  brainstorm (`brainstorm/20260627-ci-quality-gates.md`). **P1 — shipped:**
  function length (#105), unused deps (#106), and the crate-level dependency
  rule as an architecture fitness function enforcing the hexagonal inward-only
  invariant (#107) all landed. **P2 follow-ups — shipped** as the intra-crate
  refactor cluster #167–#173, now complete: the intra-crate architecture gate
  (`intra-crate-arch`) is #173 — a module-boundary check
  (`scripts/check-module-boundaries.sh`: leaf modules stay leaves, renderers
  don't reach the executor, `core::app` submodules go through the parent
  registry), an OS-cfg containment check
  (`scripts/check-os-cfg-containment.sh`: compile-time `#[cfg(target_os)]` stays
  in its audited homes, same spirit as the `unsafe_code` quarantine), and the
  report-only file-length signal — fanned into `ci-success`, mirrored by
  `just check-arch`. It followed the god-object splits #167 (shell —
  **shipped**), #168 (terminal/view — **shipped**) and #169 (core `app/` split +
  `Sessions` registry + `Sidebar`/`FontState` field-flatten, A1–A4 —
  **shipped**), plus the independent adapter splits #170 (scan →
  `watch`/`cache`/`derive`/`walk`/`repo` — **shipped**, the seam
  `F-antigravity-sessions` #160/#161 build on), #171 (F, json_store —
  **shipped**) and #172 (pty →
  `input`/`grid`/`events`/`status`/`session`/`kill`/`manager` — **shipped**, the
  seam #143 foreground-process detection and #155 vim mouse build on).
  Cognitive-complexity (signal C) stays unfiled. **P3 / report-only** (was
  blocked on a quality-report home): file length (signal A) now ships **inside
  #173's gate** as a job-summary report; churn×size hotspots (signal J) stays
  unfiled. Dropped: MSRV check,
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
  keyboard switching (`Ctrl+Tab`, via `F-keyboard-shortcuts`) both ship.
  Double-clicking a chip renames the tab inline (#145): a durable
  `Tab.custom_title` overlays the derived title (`Tab::display_title` resolves
  the precedence, so a later OSC/digest relabel never masks a custom name), a
  blank name reverts to the derived title, blur commits and Escape cancels. The
  tab-strip view was extracted to its own `shell/view/tabs.rs` for this (C2 of
  the intra-crate refactor, #168)
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
  PRD rev. 5): the **#54 MVP shipped** — the `split-*`/`focus-*`/`close-focused`
  keymap actions now drive `core` (recursive iced pane rendering from the
  `Workspace` tree, fixed-ratio 50/50 splits, per-leaf PTY geometry), plus
  click-to-focus (`Event::FocusPane`) and directional keyboard focus that
  cycles within its axis (`Event::FocusDir`, `mod+shift+arrows`); `mod+w`
  collapses the focused pane rather than the whole tab. Default binds: `mod+d`
  / `mod+shift+d` split, `mod+shift+arrows` focus. What remains is **drag-resize
  (#55, blocked-by #54)** to flip the fixed ratio; `core::Workspace` stays the
  single source of truth throughout
- [x] `F-close-on-exit` — auto-close a pane/tab on clean exit (#185, shipped
  in #187): a PTY exiting with code 0 (the user typed `exit` at a prompt)
  closes its pane — collapse the split, or close the tab (onto the reopen
  stack) when it was the last pane; an emptied workspace stays open, termherd
  never quits. Non-zero/unknown exits keep the dead-terminal view so errors
  stay readable. Quitting Claude still never closes the tab — structurally:
  `claude` is typed *into* a shell, so its exit returns to the prompt with
  the PTY alive (the planned `Launch::Claude` gate proved redundant and was
  dropped mid-review). Ship also fixed exit detection on Windows: ConPTY
  never delivers reader EOF on a child's natural exit, so the `pty` adapter
  reaps in a dedicated waiter thread. Fixed policy, no settings knob
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
- [ ] `F-antigravity-sessions` — support Antigravity (`agy`) sessions in
  TermHerd (Should):
  - **Codec / Parser (#161)**: a pure JSONL transcript parser to extract
    summary prompts, calculate message count, build text content for indexing,
    and derive CWD from tool calls.
  - **Session Discovery (#160)**: scan `~/.gemini/antigravity-cli/brain/` for
    UUID directories, parse CWD/metadata incrementally (via `ScanCache`), and
    group them.
  - **Process Spawning & UI Integration (#162)**: extend `Launch` and PTY
    spawning to run `agy` and `agy --conversation <id>`, add launch buttons
    to the sidebar, and wire up tab/sidebar display titles.

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
  control/config + orchestration surface, driven by the in-app Claude sessions
  (termherd is the server, the session is the client). Inverse of
  `F-mcp-ide-bridge`. Filed as #90 (now the **tracking epic**). Tortured 🧬
  **split** (feature-torture `F-mcp-control-surface.md`; design brainstorm
  `brainstorm/20260713-mcp-agent-terminal-interaction.md`): the entry hid
  multiple features separated by the **transport** — config is *stateless* (a
  file), orchestration/perception/synchro need the live `core::App` → an
  in-process **http/sse** server (supersedes the earlier per-session-WS
  assumption; Claude's MCP client speaks `stdio | http/sse`). A first,
  **read-only** stdio slice has landed: `crates/mcp` (`termherd-mcp`),
  `list_options` + schema resource, pure and unit-tested. Split into rungs, each
  shippable:
  - [x] `F-mcp-config-write` (#191) — `set_option` + `keys` on the stdio slice;
    independent, deliverable now
  - [x] async transport substrate (#192, `tech-health`) — tokio runtime in the
    composition root + a timeout-bounded request/reply primitive drained through
    the iced loop into `core::App` (pure state read → reply). The bound covers
    the enqueue too, so a full request channel can't hang the caller (Q7).
    Substrate-only: proven end-to-end by an in-process test transport, no live
    server yet. **Runtime = tokio** (MIT, ecosystem default; async-std is
    deprecated), feature-frugal (`rt-multi-thread`/`sync`/`time`/`macros`); the
    http/sse **server crate** pick (`tiny_http` vs `axum`/`hyper`) is deferred
    to #193, when the real transport lands and can be measured against
    MIT/no-FFI/frugal. Shared enabler, also unblocks `F-mcp-ide-bridge`;
    relates to #167/#171
  - [x] `F-mcp-live-bridge` (#193) — **the gate, green.** An in-process MCP
    server on loopback (`127.0.0.1:<ephemeral>/mcp`), reachable by the Claude
    sessions termherd launches, answering tool calls over the #192 bridge into
    the live `core::App` — core never touched directly. The `list_sessions`
    spike returns every live session with a **stable external handle** (the
    runtime `SessionId`, minted once and never re-keyed — decoupled from the
    Claude resume id that a fork/plan-accept re-keys, Q6). A **per-session bearer
    token** (v4 UUID, OS CSPRNG) gates every request, minted on each Claude
    launch and injected via the session's `mcpServers` config (a `0o600` file,
    never argv, never logged) — `Launch::Claude` carries the endpoint as opaque
    data on the spawn request; the pty writes the config and passes
    `--mcp-config`. Proven end-to-end headless (401 without a token, full
    `initialize` handshake with one, `list_sessions` through the bridge) and by
    a real app run (server binds + auth-gates on loopback). **Server crate =
    `rmcp`** (the official MCP Rust SDK — carries JSON-RPC / handshake / tool
    routing so we never hand-roll the protocol), Apache-2.0 (cargo-deny allows
    it), edition 2024, tokio-native (reuses the #192 runtime, now `enable_all`
    for the listener); the http stack is `hyper`/`hyper-util` (frugal base — no
    axum). Unblocks #194/#195/#196
  - [ ] `F-mcp-orchestration` (#194) — open/split/focus/rename/run-in-session;
    depends on #193
  - [ ] `F-mcp-orchestration` (#194) — open/split/focus/rename/run-in-session;
    depends on #193
  - [ ] `F-mcp-terminal-sync` (#195) — `wait_for_status` (OSC) + `read_terminal`;
    depends on #193
  - [ ] `F-mcp-agent-loop` (#196) — `type_into_terminal` + prompt→wait→read,
    opt-in; depends on #195; product-scope question open (may be cut)

- [x] `F-terminal-palette` — configurable terminal colours (#181, shipped
  in #183; tortured 👍, feature-torture `F-terminal-palette.md`): an optional
  `terminal.colors` block in `settings.json` — `foreground`, `background`,
  `cursor` and the 16-colour ANSI `palette`, plus a `scheme` picking a
  built-in preset (`solarized-dark`/`-light`, `gruvbox-dark`/`-light`) that
  explicit fields override. A `Palette` is injected into `PtyManager::new`
  like the shell profile — colours keep resolving in the `pty` adapter,
  `core` never sees RGB, and `Screen` carries `default_bg`/`cursor_color` so
  the canvas dropped its duplicated constants. Wide-parse per field: a bad
  value warns and degrades alone. Restart-to-apply; the MCP catalog exposes
  the five keys. Dims stay a fixed hand-tuned table (legibility guards).
  Deliberately out: selection colour (app affordance), live reload (waits
  for the in-app settings panel). Verified end-to-end via F-capture on a
  real session (Solarized Light)
- [ ] `F-session-accent-colors` — per-session / per-agent visual accents:
  give each session (or agent kind — Claude, plain shell, `agy`) a colour used
  on its tab chip, sidebar row and pane border, so parallel sessions are
  distinguishable at a glance. Chrome accents, not grid colours — sibling of,
  but separate from, `F-terminal-palette`. Natural home for the assignment is
  the `~/.termherd/metadata.json` overlay (like `F-session-metadata`).
  **Design-first**
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

- [x] `F-favorites` — favorites in the sidebar. **Designed (🧬 split,
  feature-torture `F-favorites.md`)**: "star" == "favorite" is one concept.
  Graduated to #56 (cross-project Favorites section, reusing the shipped
  session star) and #57 (repo-level favoriting, a `project_path`-keyed overlay
  in `~/.termherd/metadata.json`, never `~/.claude`). Both children
  implemented: **#57** — a `repos` map in the overlay (`RepoMeta`), a star on
  each project header that pins the group to the top, and a flat→wrapped JSON
  migration; **#56** — a cross-project "★ Favorites" section at the top of the
  sidebar aggregating every starred session (coexists with the in-group pin —
  the favourite is a shortcut, not a move). Both merged (#163, #165); the
  issues are closed. **Decision — repos in the Favorites section: 👎 killed.**
  #57's repo star already surfaces a favourited repo by pinning its group to the
  top of the sidebar; a repo row in the section would only duplicate the shipped
  🤖/`$` launch buttons (#23) or navigate to an already-top group, and mixing
  non-resumable repos into a "resume a favourite session" list breaks its model.
  Favorites stays sessions-only
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
