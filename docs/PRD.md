# TermHerd — Product Requirements Document

**Date:** 2026-06-13 (rev. 5 — `F-store-cache` (SQLite) moved to the bottom
of Should: it is an optimisation over the in-memory scan/search, not a
prerequisite for other Should items; rev. 4 2026-06-12 moved the SQLite
store from Must to Should so v1-Must search runs in-memory over scan
digests; rev. 3 same day promoted Windows + Linux to first-class; rev. 2
2026-05-27 re-prioritised to a session-workspace product)
**Status:** proposal
**Decision basis:** the `.personal/` analysis set + the owner's MoSCoW.
**Chosen path:** pure-native Rust GUI · MVP-core first · **macOS (Apple
Silicon) · Windows · Linux — all first-class at v1**.

## 1. Product framing (what changed in rev. 2)

The owner's prioritisation moves **IDE emulation and diff review to "Unsure"**
and promotes **tabs, terminal splits, keyboard shortcuts, status
notifications, and search** to Must. That reframes the product:

> TermHerd is a **terminal workspace for Claude Code sessions** — browse,
> launch, arrange (tabs + splits), monitor, and search many sessions from one
> native window. It is **not** an IDE: Claude continues to use the user's real
> editor for file edits.

This is strategically better for a pure-native restart: the **diff panel — the
single largest, hardest native build — leaves the critical path**, and the new
risk concentration (terminal multiplexing) is exactly the capability the owner
wants. Risk now aligns with value.

Dropping IDE emulation has one clean consequence: TermHerd does **not**
register in `~/.claude/ide`, so the CLI falls back to the user's configured
editor (VS Code / Cursor) — the behaviour the Electron app already exposes as a
toggle. Status notifications do **not** depend on this; they come from parsing
the terminal's own output (OSC sequences).

## 2. Goals / non-goals

### Goals

- G1 — Daily-driver fast: browse + run + arrange + monitor + search.
- G2 — Fix the four quality gaps by construction (the Quality Bar, §4).
- G3 — Native performance (low idle CPU/mem, fast start); no Electron.
- G4 — Stay a drop-in *reader* of Claude CLI files so it can run beside the
  Electron app during the parity gap.
- G5 — First-class macOS, Windows, and Linux from v1 — same features, same
  release pipeline, shipped together.

### Non-goals (v1)

- N1 — IDE emulation / in-app diff review (MCP). Deferred to "Unsure"; Claude
  uses the real editor.
- N2 — The Could features: stats, schedules.
- N3 — Reusing Electron/JS code. Domain *knowledge* is ported; code is not.

## 3. Users & jobs-to-be-done

Primary user: a power Claude Code user juggling many sessions across projects.
Re-ordered jobs (rev. 2):

- "Show me every session across all projects; find one by content; resume it."
- "Run several sessions at once and **arrange them** — tabs to switch, splits
  to watch two side by side — and drive it all from the **keyboard**."
- "Tell me which sessions need attention (busy / waiting / permission)."
- *(Removed from primary)* in-app diff review — Claude uses the real editor.

## 4. The Quality Bar

The reason for the restart. Each row maps a v0.0.30 gap to a binding,
verifiable requirement; acceptance criteria for *every* milestone.

| # | Gap in v0.0.30 | Requirement | Verified by |
| - | -------------- | ----------- | ----------- |
| Q1 | 1 test total | Core logic TDD'd; meaningful coverage on `core`/`claude` | `cargo test` + coverage in CI |
| Q2 | No CI gate | PR gate: `fmt`, `clippy -D warnings`, tests, `cargo-deny` | required GitHub checks |
| Q3 | God-object | Hexagonal workspace; core depends on no adapter | crate boundaries + CI dep check |
| Q4 | Require-time singletons | No global mutable state; deps built in `main`, injected | review + no `static mut`/`lazy_static` |
| Q5 | Silent `catch {}` | Typed errors; `unwrap`/`panic` denied in `core` | `clippy::unwrap_used` = deny |
| Q6 | `realSessionId` race | Per-session ownership + message passing | design + race-scenario tests |
| Q7\* | MCP `openDiff` hang | RPC round-trips timeout-bounded | unit test — **deferred with MCP** |
| Q8 | No single-instance lock | Startup advisory single-instance lock | integration test |
| Q9 | Loose mac entitlements | No webview ⇒ no JIT/unsigned-memory entitlements | signed build inspection |
| Q10 | Node version drift | `rust-toolchain.toml` pins toolchain | CI uses pinned toolchain |
| Q11 | Mixed logging | Single `tracing` stack | grep: no `println!` outside tests |

\* Q7 only applies if/when MCP (Unsure) is built; not in the v1 critical path.

## 5. Scope (MoSCoW)

Feature IDs carry over; three **net-new** features (not in v0.0.30) get new IDs.

### Must

| F-ID | Feature | Size | Note |
| ---- | ------- | ---- | ---- |
| `F-foundations` | workspace, core, CI, single-instance, tracing | M | enabling |
| `F-app-shell` | window, lifecycle, bounds, menu | S | hosts tabs/splits |
| `F-session-browser` | scan + derive + group + list | L | core view |
| `F-builtin-terminal` | PTY + native terminal widget | L | the product surface |
| `F-search` | in-memory search over digests | S | was `F-fts-search`; the FTS5 version is `F-store-cache` (Should) |
| `F-status-notifications` | busy / waiting / permission from OSC | M | no MCP needed |
| `F-settings` (thin) | shell select, theme, window prefs | S | full version later |
| `F-packaging-ci` | signed mac/win/linux builds + CI gate | M | `dist`, 3-OS CI matrix |
| `F-session-tabs` ⭐ | tabbed open sessions | M | **net-new** |
| `F-keyboard-shortcuts` ⭐ | configurable keymap → actions | M | **net-new** |
| `F-session-metadata` | star / rename / archive / titles | S | rev. 6: Claude-session link is core |
| `F-plans-memory` | plan + CLAUDE.md browse/edit | S | rev. 6: moved from Should |

### Should

| F-ID | Feature | Size |
| ---- | ------- | ---- |
| `F-fork-detection` | fork / plan-accept detection (rev. 7: **blocked** — 23 real sessions show no `forkedFrom`, no cross-session uuid reuse, no sub-120s transitions; resume appends in-place. Revisit if Claude reintroduces fork files) | M |
| `F-terminal-split` ⭐ | split panes (h/v), focus, resize (rev. 5: moved from Must; pure pane-tree core already landed) | L |
| `F-jsonl-viewer` | rich transcript viewer | M |
| `F-auto-update` | background update from GitHub | S |
| `F-store-cache` | SQLite (WAL) digest cache + FTS5 index — instant cold start on large trees; an optimisation over the in-memory scan/search, so it sits at the bottom of Should | M |

### Could

`F-activity-stats` (M), `F-session-grid` (S — a *layout preset* once the pane
model exists, so cheap), `F-scheduled-tasks` (M), and `F-mcp-ide-bridge` (S
protocol — rev. 6: moved up from Unsure, decoupled from the diff panel; a live
MCP/IDE link to Claude as its own adapter).

### Unsure (deferred, possibly never)

`F-file-diff-panel` (L — the largest native build). Deferring it removes the
hardest widget from v1; `F-mcp-ide-bridge`, once coupled to it, now stands
alone in Could.

### Dependency clarification (jsonl)

The owner placed "jsonl parser" in Should. Split it: the **lightweight digest
parse** (summary, titles, counts, searchable text) is required by
`F-session-browser` + `F-search`, so it lives in **`F-foundations`** (Must,
in the `claude` crate). Only the **rich transcript viewer** (`F-jsonl-viewer`)
is Should.

## 6. Functional requirements (Must)

Abbreviated acceptance criteria.

### Browser & search

- FR1 — On launch, list every session under `~/.claude/projects`, grouped by
  the real project path (derived from JSONL `cwd`, worktrees collapsed); no
  duplicate-group bug (#41/#44 class), pinned by property tests.
- FR2 — Scanning is off the UI thread; live-updates on fs changes.
- FR3 — Search over session content, case-insensitive, title-only toggle.
  v1: in-memory over scan digests (their indexed text is capped per
  session); the persisted FTS5 index arrives with `F-store-cache` (Should).

### Terminal, tabs & splits

- FR4 — Launch new / resume / reattach a session in an embedded terminal with
  correct shell + cleaned env; render ANSI, wide chars/emoji, scrollback,
  selection; resize propagates to the PTY.
- FR4a — A launch has three distinct **kinds**, decided in `core` and carried
  out by the `pty` adapter: a *plain shell* in the project dir, a *fresh
  Claude* session (`claude`, no resume), and a *resumed Claude* session
  (`claude --resume <id>`). The sidebar surfaces them per repo row as two
  explicit buttons — `$` (plain shell) and 🤖 (fresh Claude) — while clicking
  the repo *name* toggles collapse and clicking a session resumes it (#23).
  Rationale: fresh-Claude launch was a *missing* mode (every prior spawn was a
  bare shell unless a resume id was present), and overloading the repo name as
  a launch trigger was an invisible affordance.
- FR5 — Open sessions appear as **tabs**; switch, reorder, and close tabs.
- FR6 — Any tab can **split** horizontally/vertically into panes, each hosting
  a session terminal; focus moves between panes; panes resize; closing the last
  pane closes the tab.
- FR7 — Multiple sessions run concurrently without cross-talk or state races.

### Status, shortcuts, settings

- FR8 — Derive busy / waiting-for-input / permission-blocked from the terminal
  OSC stream and surface it per session in the sidebar and on the tab.
- FR9 — A **configurable keymap** drives the workspace: switch/close tab, split
  h/v, move focus, focus search, open new session — all reachable by keyboard;
  bindings persist.
- FR10 — Thin settings: shell profile, theme, window prefs; persisted.

### Shell

- FR11 — Single-instance: a second launch focuses the existing window (Q8).
- FR12 — Window bounds persist and restore.

## 7. Non-functional requirements

Mapped from the NFR scorecard; each prior gap becomes a requirement.

- **Packaging** — signed installers for all three platforms via `dist`
  (cargo-dist): macOS Apple-Silicon `.dmg`, Windows installer, Linux package
  (format OQ5); CI builds and tests on a macOS/Windows/Linux matrix.
- **Signing** — macOS: hardened runtime + notarization **without**
  JIT/unsigned-memory/library-validation entitlements (Q9) — a security
  upgrade over Electron. Windows: Authenticode (certificate sourcing is OQ5).
  Linux: signed checksums on releases.
- **Auto-update** — Should; background check/download from GitHub Releases.
- **Security** — no webview ⇒ no remote-content/CSP surface; fs access scoped
  to `~/.claude` (read) and `~/.termherd` (read/write), with one narrow,
  ADR-ratified write exception for in-app plan/memory editing — writes reach
  only `~/.claude/CLAUDE.md` and `~/.claude/plans/*.md`, never the session tree
  (see [`docs/adr/0001`](adr/0001-plans-memory-write-scope.md)); **no
  `~/.claude/ide` writes in v1** (no IDE registration); external links via OS
  opener.
- **Single-instance** — never races on the DB (Q8).
- **Performance budget** — idle CPU < 1%, idle memory < 150 MB, cold start
  < 1 s, scan of 1k sessions < 2 s off-thread; many concurrent terminals stay
  smooth (GPU-accelerated rendering).
- **Observability** — single `tracing` stack, structured, leveled (Q11).
- **Data/storage** — v1 Must persists only small config files under
  `~/.termherd/` (window, settings); the SQLite (WAL, FTS5) cache at
  `~/.termherd/` is Should (`F-store-cache`, OQ3). Reads
  `~/.claude/projects` + `~/.claude/` unchanged ⇒ coexistence with Electron.
- **Toolchain** — `rust-toolchain.toml` pins the version (Q10).

## 8. Success metrics

- M1 — Owner stops using the Electron app for the core loop: browse + run +
  tabs/splits + search (note: **not** diff review — that moves to the real
  editor) — the daily-driver switch.
- M2 — Quality Bar green (Q1–Q6, Q8–Q11) at every release.
- M3 — Performance within the §7 budget with ≥4 concurrent terminals.
- M4 — Zero `unwrap`/`panic` in `core`; CI gate never bypassed.
- M5 — No duplicate-sidebar / title-clobber regressions — each pinned by a test.

## 9. Milestones & estimate

Native rewrite midpoints: S 1.5 · M 3.5 · L 6.5 PD.

| Milestone | Delivers (F-IDs) | ~PD |
| --------- | ---------------- | --- |
| M0 — Foundation & shell | foundations, app-shell, single-instance, CI, packaging (3 OS) | 9 |
| M1 — Browser & search | session-browser, search (in-memory) | 8 |
| M2 — Terminal & status | builtin-terminal, status-notifications | 10 |
| M3 — Workspace & input | session-tabs, terminal-split, keyboard-shortcuts, settings(thin) | 13 |
| **Daily-driver switch** | end of M3 | **~40** |
| Should | store-cache, fork-detection, metadata, jsonl-viewer, plans-memory, auto-update | +14 |
| Could | activity-stats, grid, schedules | +9 |
| Unsure | file-diff-panel, mcp-ide-bridge (coupled) | +8 |

**Honesty on the budget.** The *parity* part of Must (browser, terminal,
search, status, settings, shell, packaging, foundations) is bounded by the
original ~55 PD. But tabs + splits + shortcuts are **net-new scope** the
Electron app never had (~13 PD), and shipping Windows + Linux at v1 (rev. 3)
adds ~2 PD of packaging scope to M0 — the stack itself is already
cross-platform. So Must as a whole is additive, not strictly "≤ original."
Deferring diff+MCP (Unsure, ~8 PD) and the SQLite store (rev. 4, Should)
is what keeps M0–M3 at ~40 PD and pulls the hardest widget off the
critical path.

## 10. Risks & mitigations

| Risk | Sev | Mitigation |
| ---- | --- | ---------- |
| **Terminal multiplexing** (tabs/splits/focus over N live PTYs) is the new hard core | High | Build the pane model as a pure tree in `core` (testable); render with `alacritty_terminal`; M3 is time-boxed and split-resize can ship before drag-resize |
| **Parity gap** — no daily driver until M3 | High | MVP ordering; run beside Electron (shared `~/.claude` reads) until M3 |
| Native terminal widget cost | Med | `alacritty_terminal` does ANSI/grid; widget is render+input |
| Scope creep from net-new features | Med | MoSCoW is the contract; Could/Unsure stay out of M0–M3 |
| Claude format/OSC drift | Med | isolated, property-tested `claude` crate |
| **3-OS surface at v1** (PTY, shells, fs paths, signing differ per platform) | Med | stack chosen cross-platform (`portable-pty`, `notify`, `iced`, `dist`); platform code isolated in adapters; CI matrix on all 3 OSes from M0 so drift is caught per-commit, not at release |

## 11. Open questions

- OQ1 — Final GUI crate: `iced` (recommended — Elm/testable, the pane tree maps
  cleanly to it) vs `egui` (faster MVP) vs `GPUI` (best terminal primitives).
- OQ2 — Keymap config format (TOML keymap file? defaults + overrides?).
- OQ3 — Storage: fresh `~/.termherd` schema + one-time import from the
  Electron DB, or read the existing schema? (Recommend: fresh + import.)
- OQ4 — Do tabs and splits share one focus model with the Could `F-session-grid`
  (grid = a saved layout preset)? (Recommend: yes — design once.)
- OQ5 — Windows code-signing certificate sourcing, and the Linux package
  format (AppImage vs `.deb`/`.rpm` vs tarball + shell installer)?
  (Recommend: start with cargo-dist defaults — tarball/shell + `.msi` — and
  add formats on demand.)
