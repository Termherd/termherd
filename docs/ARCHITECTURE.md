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
| Async runtime | **tokio** | PTY read loops, fs watch, the MCP servers | Must |
| Errors / logging | **thiserror**/`anyhow` + **tracing** | typed errors, structured logs | Must |
| Release/update | **dist** (CLI archives) + **cargo-packager** (installers) | one GitHub release; installers unsigned for now and no self-update yet (§14) | Must (update = Should) |
| IDE bridge (termherd as MCP client, §7) | tokio-tungstenite | per-session WS + JSON-RPC | **Deferred (Unsure)** |
| MCP servers (live bridge, stdio) | `rmcp` (live bridge); `serde_json` only (stdio) | termherd as MCP *server* (§7, §15) | Shipped |
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
   ┌──────────┐ ┌──────────┐ ┌──────────┐
   │crates/   │ │crates/   │ │crates/   │
   │ pty      │ │ scan     │ │ store    │
   │portable- │ │ notify + │ │ (unbuilt)│
   │pty+alacr.│ │ rayon    │ │          │
   └──────────┘ └──────────┘ └──────────┘
   crates/mcp and crates/private-fs also point inward but implement no
   port (roles in §15).
   crates/app builds the concrete adapters in main() and injects them.
```

- `core` defines port traits (`SessionStore`, `PtyHost`, `ProjectScanner`,
  `PathResolver`, `Clock`, ~~`FileSystem`~~) and never names a concrete
  adapter. `PathResolver` landed with clickable file paths (#252): telling
  `src/main.rs` from prose like `and/or` needs the filesystem, which `core` may
  not touch. **`FileSystem` never existed** — it was sketched here and never
  written, so a reader looking for it is not missing something, and the file
  read that would have used it goes through `ProjectScanner`.
- ~~`mcp` exists in the workspace only if/when the Unsure bet is taken; nothing
  in the v1 path imports it.~~ **Superseded.** The name now belongs to the
  shipped MCP **control surface** (`F-mcp-control-surface`, #90): its live
  bridge is a module inside `app` (§7, §8), and `crates/mcp` holds its stdio
  server and the option catalogue both servers share (§15). What stays
  deferred is the IDE bridge (`IdeBridge`, termherd as an MCP **client**,
  §7), which has no crate and no port yet.
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
- **`scan`** (`ProjectScanner`, `PathResolver`) — `notify` watches
  `~/.claude/projects` (debounced); `rayon` parses folders in parallel off the
  UI thread; uses the `claude` codec. It also resolves the file paths a
  terminal prints (#252): a candidate is tried against the session's live
  `cwd`, the repository holding it, then its launch directory, and judged on
  the symlink-followed form — the name says nothing about what an OS opener
  would reach.
- **`claude`** (pure) — ported domain knowledge: lossy folder encode/decode,
  `cwd`-based path derivation + worktree collapse, JSONL → digest parse, OSC
  decode, fork/plan-accept signals. No I/O ⇒ unit- and property-tested.
- **IDE bridge** (`IdeBridge`) — *deferred (Unsure)*. If built: a
  per-session WS server with a `~/.claude/ide` lock and JSON-RPC,
  `tokio::timeout` on every round-trip (Q7). Not in the v1 path.

  **Not to be confused with the shipped MCP control surface** (#90). That one
  runs the other way round — termherd is the *server* and the Claude sessions
  it hosts are the clients. The live bridge lives in `app` (`app::mcp` over
  `shell::bridge` / `shell::serve`), because it is a bridge *into* the shell
  rather than a port `core` calls *out* through; what `crates/mcp` holds is
  in §15. The `tokio::timeout`-on-every-round-trip rule (Q7) is shared: it is
  enforced in `BridgeHandle::call`.

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
   re-renders. `notify` triggers incremental re-scans. The rendered membership
   is not the scan alone: declared repositories come from the `metadata.json`
   overlay and are united with the scanned groups on the same key, so a rebuild
   has a second input that no scan event carries.

### Open a session into the workspace

1. Key chord / click → `core` resolves an `Action::OpenSession` → `apply`
   inserts a leaf in the active tab (or a new tab) and returns `Effect::Spawn`.
2. `app` spawns via `pty`; a read-loop task feeds bytes →
   `alacritty_terminal` → `Event::TerminalBytes` + `Event::OscStatus`.
3. The terminal widget for that pane renders the grid; the tab/sidebar show the
   OSC-derived status.

Activity (FR8) has **two dialects and a stand-in**, because two kinds of session
are hosted. A Claude session announces itself in the CLI's own OSC stream (glyph
titles, OSC 9); a plain shell announces nothing of the sort, so `pty` injects an
OSC 133 shell-integration snippet and decodes the prompt/command marks it emits.
Both fold into the same `SessionStatus`. A shell whose integration did not take
(an unknown shell, an unwritable temp directory) falls back on the PTY's
foreground process group — a stand-in the first real signal or mark retires for
good, since it would otherwise call every Claude session busy forever. ConPTY
exposes no foreground group, so that fallback is silent on Windows. The job in
front is read there off the process tree below the shell instead (the nearest
`claude.exe`, through the Win32 FFI in `pty::status::win_process`), so a
Claude's session file is still found; no other program is reported, since a
child of the shell need not be in front of it.

The same injected snippet answers a second question at the same prompt hook:
**where** the shell is. It announces its directory as an OSC 7 `file://` url,
which `pty` decodes into `Event::SessionCwdChanged`, so `LiveSession.cwd`
follows a `cd` instead of freezing on the launch directory — and the four
readers of it — the MCP / capture snapshot, the directory a split inherits,
the "new shell / new Claude here" shortcuts, and the tab card — stop reporting
where the session started. A
shell with no recipe announces nothing and keeps its launch directory, the same
degradation as its status.

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

A tag push runs two pipelines onto one GitHub Release; `docs/CI.md` holds
their job graphs.

- **`dist` (cargo-dist), `release.yml`:** CLI archives and shell/PowerShell
  installers for five targets. No updater is installed
  (`install-updater = false`); self-update is `F-auto-update`, unbuilt.
- **`cargo-packager`, `package.yml`:** the desktop installers — `.app` and
  `.dmg` on macOS, `.deb` and AppImage on Linux, NSIS on Windows.
- **Signing:** `sign-release.yml` publishes `SHA256SUMS` and a keyless
  Sigstore provenance attestation over the Linux archives. The macOS and
  Windows installers ship **unsigned** for now (certificate sourcing OQ5).
  The target stays: macOS hardened runtime + notarization with **no**
  JIT/unsigned-memory/library-validation entitlements (Q9), Windows code
  signing.
- CI builds and tests on a 3-OS matrix from M0.
- Toolchain pinned via `rust-toolchain.toml`; local packaging steps and the
  architecture gates live in the `justfile` (not an `xtask`), mirroring
  `package.yml` and the matching `ci.yml` jobs.

## 15. Module / responsibility map

| Crate | Responsibility | Key deps | Status |
| ----- | -------------- | -------- | ------ |
| `core` | domain + headless `App` + workspace + keymap + ports | `claude` | Must |
| `claude` | Claude CLI format codec (pure) | — | Must |
| `store` | SQLite cache, metadata, FTS5 | `rusqlite` | Should (rev. 4), unbuilt |
| `pty` | PTY spawn + terminal grid/OSC, per-session task | `portable-pty`, `alacritty_terminal` | Must |
| `scan` | fs scan + watch | `notify`, `rayon` | Must |
| `app` | iced GUI: sidebar, tabs, splits, terminal; live MCP bridge | `iced` | Must |
| `mcp` | stdio MCP server binary + settings-option catalogue | `core` | Shipped |
| `private-fs` | owner-only files and directories | — | Shipped |
| IDE bridge | per-session IDE/MCP WS server (termherd as MCP client, sketched in §7); no crate yet, and not `mcp` | `tokio-tungstenite` | Deferred |

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
  is render + input; fixed-ratio splits shipped first, and drag-resize now
  updates the same pane tree.
- **Native terminal widget cost** (selection, scrollback, unicode width).
  Mitigation: lean on `alacritty_terminal`; budgeted in `F-builtin-terminal`.
- **GUI framework churn.** Mitigation: logic is in `core`; the GUI crate is
  replaceable without touching tests or adapters.
- **Claude format/OSC drift.** Mitigation: isolated, property-tested `claude`.
- **The Unsure bet (IDE bridge/diff) returning later.** Mitigation: it would
  enter as a new port in `core::ports` and an adapter implementing it —
  additive, not a rework. No `IdeBridge` port exists yet; it is written when
  the bet is taken, not ahead of it.
