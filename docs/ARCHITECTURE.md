# TermHerd — Architecture

**Date:** 2026-06-12 (rev. 3 — Windows + Linux first-class at v1; rev. 2
2026-05-27 session-workspace product, IDE deferred)
**Status:** proposal (pairs with the Rust PRD)
**Target:** pure-native Rust, macOS (Apple Silicon) + Windows + Linux — all
first-class at v1 — MVP core then iterate.
**North star:** keep the product's value; fix the four quality gaps *by
construction*. Every choice is justified against a gap from the review.

## 1. What rev. 2 changes

The product is now a **terminal workspace for Claude sessions**, not an IDE.
Architecturally:

- The **MCP/IDE server** and the **diff panel** drop out of the v1 critical
  path (Unsure) — removing an entire adapter and the hardest widget.
- The new hard core is **terminal multiplexing**: tabs + split panes + a
  keyboard-driven workspace over N concurrent PTYs.
- That multiplexing is modelled as **pure data in `core`** (a pane tree + a
  keymap), which is the best possible fit for the headless-core/TDD approach.

The hexagonal skeleton from rev. 1 is unchanged and is, if anything, *more*
validated: a session workspace is squarely in pure-native-Rust territory.

## 2. Architectural goals

1. **No god-object** — logic in a GUI-free, I/O-free core; the world is behind
   traits. (Q3)
2. **No hidden singletons** — deps built in `main`, injected. (Q4)
3. **No data races, by ownership** — per-session state owned by one task,
   mutated only via messages. (Q6)
4. **Predictable failure** — typed errors; no `unwrap`/`panic` in core. (Q5)
5. **Testable to the bone** — application logic, the pane tree, and the keymap
   all run headless, with no GUI/PTY/network. (Q1)

## 3. Tech-stack decisions

| Concern | Choice | Why | Status |
| ------- | ------ | --- | ------ |
| GUI | **iced** | Elm arch → pure `update`/`view`, testable; the pane tree maps cleanly; wgpu-accelerated | Must |
| Terminal grid | **alacritty_terminal** | de-facto ANSI/VTE state machine + grid; also exposes OSC for status | Must |
| PTY process | **portable-pty** | cross-platform spawn/resize, cleaned env | Must |
| Storage | **rusqlite** (bundled) + FTS5 | embedded SQLite, WAL, full-text; no entitlement cost | Must |
| FS scan/watch | **notify** + **rayon** | debounced watch + parallel scan off the UI thread | Must |
| Async runtime | **tokio** | PTY read loops, fs watch, (later) MCP | Must |
| Errors / logging | **thiserror**/`anyhow` + **tracing** | typed errors, structured logs | Must |
| Release/update | **dist** (cargo-dist) + axoupdater | signed installers for mac/win/linux, GitHub release, self-update | Must (update = Should) |
| MCP/IDE server | tokio-tungstenite | per-session WS + JSON-RPC | **Deferred (Unsure)** |
| Diff / highlight | `similar` + `syntect` | diff + syntax for the panel | **Deferred (Unsure)** |

Tabs, splits, and the keymap need **no new crates** — they are pure logic in
`core`.

## 4. Crate workspace & the dependency rule

Cargo workspace, hexagonal. The **dependency rule**: everything points inward
to `core`; `core` depends only on the pure `claude` codec. This is what
structurally prevents a new god-object.

```text
                       ┌──────────────────────────────────┐
                       │  crates/app   (bin: termherd)     │
                       │  iced GUI — tab bar, split panes,  │
                       │  terminal widget; thin translator  │
                       └─────────────────┬─────────────────┘
                                         │ depends on
                       ┌─────────────────▼─────────────────┐
                       │  crates/core   (lib)              │
                       │  • domain model                    │
                       │  • headless App state machine      │
                       │  • workspace: pane tree + tabs      │  ← depends on
                       │  • keymap → action dispatch         │     nothing
                       │  • ports (traits)                   │     concrete
                       │  • depends only on ▼ claude          │
                       └─────────────────┬─────────────────┘
                                         │ uses (pure)
                       ┌─────────────────▼─────────────────┐
                       │  crates/claude  (lib, pure)        │
                       │  path encode/derive, JSONL digest, │
                       │  transition signals, OSC decode     │
                       └────────────────────────────────────┘

   adapters implement core::ports and depend on core (not the reverse):
   ┌──────────┐ ┌──────────┐ ┌──────────┐    ┌───────────────────┐
   │crates/   │ │crates/   │ │crates/   │    │ crates/mcp        │
   │ store    │ │ pty      │ │ scan     │    │ (DEFERRED/Unsure) │
   │ rusqlite │ │portable- │ │ notify + │    │ tokio ws jsonrpc  │
   │ + FTS5   │ │pty+alacr.│ │ rayon    │    └───────────────────┘
   └──────────┘ └──────────┘ └──────────┘
   crates/app builds the concrete adapters in main() and injects them.
   xtask/  — build & release automation (replaces npm scripts)
```

- `core` defines port traits (`SessionStore`, `PtyHost`, `ProjectScanner`,
  `Clock`, `FileSystem`) and never names a concrete adapter.
- `mcp` exists in the workspace only if/when the Unsure bet is taken; nothing in
  the v1 path imports it.
- The rule is CI-enforceable, so "main.js → 1,461 LOC" cannot recur.

## 5. The headless core (the quality keystone)

The Elm architecture lives in `core`, not the GUI. `core` exposes a headless
application driven by events, returning effects:

```rust
// crates/core — illustrative
pub struct App { workspace: Workspace, projects: Projects, /* … */ }

pub enum Event {
    ScanCompleted(Vec<ProjectDigest>),
    TerminalBytes { session: SessionId, data: Bytes },
    OscStatus { session: SessionId, status: SessionStatus }, // busy/idle/...
    Key(KeyChord),                 // routed through the keymap
    UserRenamed { session: SessionId, name: String },
    SessionExited { session: SessionId },
    // …
}

pub enum Effect {
    Spawn { session: SessionId, opts: SpawnOpts },
    Resize { session: SessionId, cols: u16, rows: u16 },
    PersistName { session: SessionId, name: String },
    // …
}

impl App {
    /// Pure: no I/O, no clock, no panic. Unit/property testable.
    pub fn apply(&mut self, ev: Event) -> Vec<Effect> { /* … */ }
}
```

Why this fixes the gaps: `apply` is a pure function (Q1, Q5); logic is
data-driven, not a pile of IPC handlers (Q3); state is single-owner so there is
no race (Q6); effects are *described*, not performed, so error handling is
explicit at the boundary that performs them.

## 6. Workspace, tabs/splits & input (the new core domain)

The capability the owner most wants is modelled as **pure data in `core`** —
the highest-value TDD target in the whole system.

```rust
// crates/core::workspace — illustrative
pub enum Pane {
    Leaf(SessionId),
    Split { dir: SplitDir, ratio: f32, a: Box<Pane>, b: Box<Pane> },
}
pub struct Tab { root: Pane, focus: PaneId, title: String }
pub struct Workspace { tabs: Vec<Tab>, active: usize }

impl Workspace {
    pub fn split(&mut self, dir: SplitDir) { /* … */ }     // pure
    pub fn close_focused(&mut self) { /* … */ }            // pure
    pub fn focus_next(&mut self) { /* … */ }               // pure
    pub fn open_tab(&mut self, s: SessionId) { /* … */ }   // pure
}
```

- **Tabs/splits** are tree edits — deterministic, exhaustively testable (split,
  close, focus traversal, ratio bounds, "closing last pane closes the tab").
- **Keymap** is a pure `KeyChord -> Action` map (defaults + user overrides from
  a TOML file, OQ2); `apply` dispatches `Action`s the same way as any `Event`.
- `F-session-grid` (Could) becomes a **saved `Workspace` layout preset** — no
  new model, which is why it is cheap (OQ4).

The GUI never owns layout logic; it renders the `Workspace` and forwards key
chords. This keeps the multiplexer — the riskiest part — fully testable without
a window.

## 7. Adapters (the outside world)

Each adapter implements a `core` port and fixes a specific v0.0.30 defect.

- **`store`** (`SessionStore`) — *Should, `F-store-cache` (PRD rev. 4)* —
  rusqlite, WAL, FTS5; migrations via `refinery`; constructed in `main`,
  injected — not a require-time singleton (Q4); every query returns
  `Result` (Q5). v1 Must runs from in-memory scan results.
- **`pty`** (`PtyHost`) — `portable-pty` spawns/resizes each session's process
  with a cleaned env; `alacritty_terminal` parses bytes into a grid and surfaces
  **OSC status** (busy/idle/notification) as typed events — replacing fragile
  regex-on-stream. **One owning task per session**, so N terminals across panes
  never share mutable state.
- **`scan`** (`ProjectScanner`) — `notify` watches `~/.claude/projects`
  (debounced); `rayon` parses folders in parallel off the UI thread; uses the
  `claude` codec.
- **`claude`** (pure) — ported domain knowledge: lossy folder encode/decode,
  `cwd`-based path derivation + worktree collapse, JSONL → digest parse, OSC
  decode, fork/plan-accept signals. No I/O ⇒ unit- and property-tested.
- **`mcp`** (`IdeBridge`) — *deferred (Unsure)*. If built: a per-session WS
  server with a `~/.claude/ide` lock and JSON-RPC, `tokio::timeout` on every
  round-trip (Q7). Not in the v1 path.

## 8. GUI shell (iced)

`crates/app` is intentionally thin:

- `Model` holds (or mirrors) `core::App`, including the `Workspace`.
- `Message` = GUI events + adapter notifications.
- `update`: translate `Message` → `core::Event`, call `app.apply`, turn
  returned `Effect`s into `Task`s (spawn/resize PTY, persist). Mechanical.
- `view`: render the sidebar, the **tab bar**, the **split-pane tree**, and a
  terminal widget per leaf (drawing the `alacritty_terminal` grid).
- `subscription`: bridge tokio mpsc channels (PTY bytes, OSC status, fs
  changes, scan results) into `Message`s.

Swapping iced for egui/GPUI (OQ1) touches only this crate.

## 9. Concurrency & ownership

- One **tokio** runtime hosts a read-loop task per PTY and the fs watcher.
- **Actor-per-session:** each session is owned by a task holding its PTY handle
  and terminal grid; other parts talk to it only via channels. No shared
  `&mut Session`.
- This is how the `realSessionId` race (Q6) disappears: re-keying on fork is a
  message to the owning task, processed in order — the racy version fails to
  compile.
- The GUI thread owns `core::App`; adapter events arrive as messages, keeping
  `apply` single-threaded and deterministic even with many live terminals.

## 10. Key data flows

### Session discovery → display

1. `scan` (rayon) reads folders; `claude` derives the real path and parses each
   JSONL → digest. Parse errors are logged and skipped, never panic.
2. Digests upserted into `store` (SQLite + FTS5) in a transaction — once
   `F-store-cache` (Should) lands; v1 Must keeps them in memory.
3. `scan` emits `ScanCompleted`; `apply` rebuilds grouped projects; the sidebar
   re-renders. `notify` triggers incremental re-scans.

### Open a session into the workspace

1. Key chord / click → `core` resolves an `Action::OpenSession` → `apply`
   inserts a leaf in the active tab (or a new tab) and returns `Effect::Spawn`.
2. `app` spawns via `pty`; a read-loop task feeds bytes →
   `alacritty_terminal` → `Event::TerminalBytes` + `Event::OscStatus`.
3. The terminal widget for that pane renders the grid; the tab/sidebar show the
   OSC-derived status.

### Tabs / splits / focus

Pure `Workspace` edits in `core` (split, close, focus-next, reorder). The only
side effect is `Effect::Resize` to the affected PTYs when a split changes pane
geometry. Fully testable without a GUI.

### IDE diff review (MCP) — deferred

If the Unsure bet is taken: CLI → WS `openDiff` → `Event::DiffRequested` → diff
panel → `Effect::ReplyDiff`, with a timeout bound (Q7). Until then,
TermHerd does **not** register as an IDE and Claude uses the real editor.

## 11. Data, storage & Claude compatibility

- App data: `~/.termherd/` SQLite (WAL, FTS5); fresh schema + one-time import
  from the Electron app's DB (`~/.switchboard/switchboard.db`, OQ3).
- Reads `~/.claude/projects/**` and `~/.claude/` unchanged. **v1 writes nothing
  under `~/.claude/ide`** (no IDE registration) — one less coordination point.
- **Coexistence:** because reads are unchanged, the Rust app runs beside the
  Electron app during the parity gap. Bonus: if the owner still wants in-app
  diffs meanwhile, the *Electron app* can keep providing IDE emulation while the
  Rust app provides the workspace.

## 12. How each quality gap is structurally fixed

| Gap (v0.0.30) | Structural fix |
| ------------- | -------------- |
| God-object `main.js` | hexagonal workspace + dependency rule (§4) |
| Require-time singletons | construct-in-`main` + inject ports (§7, Q4) |
| ~12 silent `catch {}` | typed `Result`; no `unwrap`/`panic` in core (Q5) |
| `realSessionId` race | actor-per-session + ownership (§9, Q6) |
| MCP `openDiff` hang | `tokio::timeout` — *only when MCP is built* (Q7) |
| No single-instance lock | advisory lock at startup (Q8) |
| Loose mac entitlements | no webview/native modules ⇒ tight runtime (Q9) |
| 1 test, untestable design | pure headless core + pane tree + keymap tests (Q1) |
| No CI gate | fmt/clippy/test/deny required checks (§14, Q2) |
| Node version drift | `rust-toolchain.toml` (Q10) |
| Mixed logging | one `tracing` stack (Q11) |

## 13. Testing strategy

- **Unit (core):** `apply(Event) -> Effect` cases — the bulk of logic.
- **Workspace (core):** exhaustive pane-tree tests (split/close/focus/ratio,
  "last pane closes tab") and keymap dispatch — the riskiest feature, fully
  testable headless.
- **Property (claude):** `proptest` round-trips for path encode/derive; fuzzed
  JSONL never panics; title-precedence invariants (the #46 contract).
- **Integration (adapters):** `store` against a tmp SQLite; `scan` against tmp
  `~/.claude` fixtures (the pattern the Electron app's one test used); `pty`
  against a fake process emitting known ANSI/OSC.
- **CI gate (Q2):** `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo nextest run`, coverage, `cargo deny check`; required on PR.

## 14. Build, release, signing, update

- **`dist` (cargo-dist):** one release pipeline for all three platforms —
  mac-ARM signed `.dmg`, Windows installer, Linux package (format OQ5) — on
  GitHub Releases with self-update; the Rust-native analogue of
  electron-builder + electron-updater.
- **macOS:** hardened runtime + notarization, **no** JIT/unsigned-memory/
  library-validation entitlements (Q9). **Windows:** Authenticode signing
  (certificate sourcing OQ5). **Linux:** signed checksums. CI builds and
  tests on a 3-OS matrix from M0.
- Toolchain pinned via `rust-toolchain.toml`; custom steps (icons, dmg layout)
  live in `xtask`, keeping CI YAML thin.

## 15. Module / responsibility map

| Crate | Responsibility | Key deps | Status |
| ----- | -------------- | -------- | ------ |
| `core` | domain + headless `App` + workspace + keymap + ports | `claude` | Must |
| `claude` | Claude CLI format codec (pure) | — | Must |
| `store` | SQLite cache, metadata, FTS5 | `rusqlite` | Should (rev. 4) |
| `pty` | PTY spawn + terminal grid/OSC, per-session task | `portable-pty`, `alacritty_terminal` | Must |
| `scan` | fs scan + watch | `notify`, `rayon` | Must |
| `app` | iced GUI: sidebar, tabs, splits, terminal | `iced` | Must |
| `xtask` | build/release automation | — | Must |
| `mcp` | per-session IDE/MCP WS server | `tokio-tungstenite` | Deferred |

## 16. Migration path from the Electron app

1. **M0–M1:** Rust app reads the same `~/.claude/projects`; browse + search
   only. Runs beside Electron; zero risk.
2. **M2:** single terminals + status. Use Rust for new sessions; keep Electron
   for anything mid-flight (and for IDE diffs, which Rust won't do).
3. **M3:** tabs + splits + shortcuts land; **switch daily driver**; one-time DB
   import brings stars/renames/archives across.
4. **Post-M3:** Should/Could features; reconsider the MCP/diff Unsure bet only
   if the real-editor fallback proves insufficient.

## 17. Architecture-specific risks

- **Terminal multiplexing is the new hard core.** Mitigation: the pane tree is
  pure `core` data (testable); `alacritty_terminal` does ANSI/grid; the widget
  is render + input; split-resize can ship before drag-resize.
- **Native terminal widget cost** (selection, scrollback, unicode width).
  Mitigation: lean on `alacritty_terminal`; budgeted in `F-builtin-terminal`.
- **GUI framework churn.** Mitigation: logic is in `core`; the GUI crate is
  replaceable without touching tests or adapters.
- **Claude format/OSC drift.** Mitigation: isolated, property-tested `claude`.
- **The Unsure bet (MCP/diff) returning later.** Mitigation: the `IdeBridge`
  port already exists in `core`; adding the adapter is additive, not a rework.
