# AGENTS.md

## What this is

`termherd` is a Rust replatform of an Electron Claude Code session
manager. The product is a **terminal workspace for Claude Code sessions** —
browse, launch, arrange (tabs + splits), monitor, search — driven from the
keyboard, on macOS, Windows, and Linux (all three first-class). The restart
exists to fix four quality gaps
(god-object, races, silent catches, untestable design) **by construction**.

Authoritative design lives in `docs/PRD.md` and `docs/ARCHITECTURE.md`. Read
them before any non-trivial work — the constraints below are downstream of
them.

## Commands

```bash
cargo run -p termherd-app          # run the binary (M0: tracing + single-instance stub)
cargo test --workspace             # all tests
cargo test -p termherd-core        # tests for one crate
cargo test -p termherd-core workspace::tests::split_wraps_leaf  # one test by path

# CI gates — mirror locally before pushing (CI runs all of these and they are blocking)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace             # CI uses `cargo nextest run --workspace`
cargo deny check                   # if cargo-deny installed
cargo machete                      # unused deps; if cargo-machete installed
just check-deps                    # hexagonal crate dependency rule (deps point inward)
just check-arch                    # intra-crate module boundaries + OS-cfg containment (+ length report)

# Markdown is also gated in CI
markdownlint-cli2                  # uses .markdownlint-cli2.jsonc

# Planning hygiene — not a CI gate (needs a `project`-scoped token)
just board-check                   # open issues the board hasn't classified
```

Toolchain is pinned to **Rust 1.95.0 / edition 2024** via `rust-toolchain.toml`
(Q10) — do not bump without updating the pin.

CI runs each gate **only when its file category changed** (a `changes` job with
`dorny/paths-filter`): a docs-only PR skips every Rust job, a pure-`.rs` change
skips the dependency-metadata jobs. All gates fan into one required check,
`ci-success`, which treats path-skipped jobs as passing — so `main` branch
protection pins that single check. Gate any new job on its category; never make
a path-filtered job a *required* check directly.

Full CI reference — every gate, its goal, when it runs, how to mirror it, and
the sanctioned exceptions — lives in [`docs/CI.md`](docs/CI.md).

### Running & observing a build

Some behaviour is GUI/OS-level and **cannot be exercised by a headless test**
— the macOS Cmd+Q quit-confirm flow, window placement, the PTY canvas. Verify
those by running the app and reading its `tracing` output:

```bash
# `tracing` is the only observation channel — there is no `println!`. Raise the
# level with RUST_LOG (default is `info,…`, see `DEFAULT_FILTER` in main.rs).
RUST_LOG=info cargo run -p termherd-app

# Add log lines at the seam you're verifying (info!/warn!, never println!), run,
# and grep the output for them — e.g. the quit path logs `request_quit`'s branch
# and the macOS menu repoint.
```

The app is **single-instance** (an flock at `std::env::temp_dir()/…`). To run a
build *alongside* one that already holds the lock — common, since a dev/agent
session often runs *inside* a release `TermHerd.app` you can't quit — point the
new process at a throwaway temp dir so its lock path differs:

```bash
TMPDIR=$(mktemp -d) RUST_LOG=info cargo run -p termherd-app   # second instance
```

`temp_dir()` honours `$TMPDIR`, so both run. Launch detached when you need to
keep interacting with the original window (e.g. to compare quit behaviour).

### Capturing state for the AI dev loop (#108)

Press **⌘⇧S** (macOS) / **Ctrl+Shift+S** (rebindable as `capture`) to dump the
running app's state for an AI assistant to read — rung 0+1 of `F-capture`. Each
press writes a timestamped pair to `~/.termherd/captures/`:

- `capture-<ts>.json` — a diffable state dump of the whole workspace: focus,
  resolved config, the sidebar, every tab with its panes (each pane's stable
  handle, kind, cwd, status), and the focused terminal's visible text. No vision
  needed.
- `capture-<ts>.png` — the real window pixels (iced `window::screenshot`), for
  render / colour / glyph bugs the text dump can't show.

The dump **is** the `WorkspaceSnapshot` the MCP `snapshot` tool reports, under a
fixed full filter (`SnapshotFilter::capture()`) — one model, two readers, so a
field never means one thing on disk and another on the wire.

`<ts>` is a UTC `YYYYMMDD-HHMMSS-mmm` stamp, so the **latest capture is the
highest-named pair** — an AI finds it by sorting the directory. Capture stays
pure in `core` (`Event::Capture(SnapshotInputs)` →
`Effect::Capture(WorkspaceSnapshot)`); all I/O — the clock, JSON/PNG encoding,
the files — lives in the `app` adapter (`crates/app/src/capture.rs`), which
shares its wire form with the MCP handler (`crates/app/src/snapshot_dto.rs`).

For motion (rung 2, #124), press **⌘⇧R** / **Ctrl+Shift+R** (rebindable as
`toggle-record`) to start a **GIF screencast**; press again to stop, or let it
auto-stop at the cap (default 8 fps / 30 s / 0.5× scale, set under a `record`
block in `settings.json` — #127). It writes `capture-<ts>.gif` to the same dir.
Same hexagonal split: `core` owns the
idle→recording state machine (frames are the time proxy — no clock), and the
`gif` encoder runs on a dedicated thread in `app` (`crates/app/src/record.rs`)
so the UI — and the recording — stay smooth.

### Driving termherd over MCP (#90)

A Claude session **launched from termherd** gets an in-process MCP server wired
into its `mcpServers` at spawn (loopback, per-session token) — so it can read
and drive the workspace it runs in. This is the richer sibling of the capture
dump above: same `WorkspaceSnapshot` model, live instead of a file.

**Settled.** Seven slices shipped: `list_sessions` + `snapshot` (perception),
`open_session` / `split_pane` / `focus_pane` / `rename_tab` / `close_pane` /
`run_in_session` (action), `wait_for_status` + `read_terminal`
(synchronisation), `screenshot` (pixels). The loop they exist to serve is
**act → wait → observe**: `run_in_session` returns immediately, so synchronise
with `wait_for_status` and then `read_terminal`. Do **not** poll `snapshot` in
a loop — it races the transition you are watching for, which is why the wait
rung exists.

`screenshot` is the pixel companion to the text `snapshot`, for the render,
colour and glyph questions text cannot answer. Reach for it *last*: a
default-bound window is ~200 kB of PNG and a third more again as base64, where
a `snapshot` is a few hundred bytes. Two bounds keep that honest — `max_width`
(default 1200) and a total-pixel ceiling for tall windows a width alone never
reaches — and a window smaller than them is never upscaled. Lower `max_width`
when a coarse view will do. A headless run has no window and says so as a
tool-level error; the text reads keep working.

Sessions are addressed by a stable `handle` (the runtime `SessionId`), never
the Claude `resume_id`, which re-keys on a fork / plan-accept (Q6). Every call
is `tokio::timeout`-bounded in `BridgeHandle::call` (Q7).

Where it lives: tools in `crates/app/src/mcp/handler.rs`, transport in
`shell::bridge`, and the shell's answers in `shell::serve` — the one place an
external caller meets `core::App`. `core` has no MCP awareness at all; every
mutation goes through an existing `Event`.

**Still open.** `F-mcp-agent-loop` (#196 — the composed prompt→wait→read in one
round trip) and `F-mcp-keys` (#229 — key chords into the *app*, so the palette,
the browser and any binding become reachable). Both on the #90 epic. With
`screenshot` they are one capability in three parts: drive the UI, see the
pixels, read the terminal — what lets an agent verify a gesture fix instead of
only proposing it.

**Looks like a contradiction, is not.** `docs/ARCHITECTURE.md` §15 lists an
`mcp` crate as *deferred (Unsure)*. That is a **different feature** —
`F-mcp-ide-bridge`, termherd as an MCP *client* of Claude's IDE bridge — and it
really is unbuilt. The surface described here runs the other way round
(termherd is the server) and lives in `app`, not in a crate of its own.

## Architecture — the dependency rule

Hexagonal workspace. The single most important invariant:

```text
app  ──►  core  ◄──  adapters          (adapters depend on core, never reverse)
           │
           ▼
         claude   (pure codec; no I/O)
```

- `crates/core` — domain, headless `App` state machine, `Workspace` (pane
  tree + tabs), keymap, port traits. **Depends only on `claude`.** No I/O, no
  globals, no `unwrap`/`expect`/`panic` (these are clippy-denied here, see
  `crates/core/Cargo.toml`).
- `crates/claude` — pure Claude CLI format codec (path encode/derive, JSONL
  digest, OSC decode). Same strict lint profile as `core`.
- `crates/app` — iced GUI shell. Constructs the adapters in `main()` and
  injects them into `core::App`; owns the one effect executor
  (`shell::effects`) and the MCP control surface (`app::mcp` + `shell::bridge`
  / `shell::serve`).
- `crates/scan` — filesystem discovery adapter (walks `~/.claude/projects`
  via the `claude` codec; implements `core::ports::ProjectScanner`).
- `crates/pty` — terminal adapter (`portable-pty` + `alacritty_terminal`);
  implements `core::ports::PtyHost`.
- `store` (Should, PRD rev. 4) is the one adapter still unbuilt. The **MCP
  control surface shipped** as a module inside `app`, not as its own crate —
  it is a bridge into the shell, not a port `core` calls out through. The
  separate `mcp` *crate* sketched in `docs/ARCHITECTURE.md` §15 is a different
  feature (`F-mcp-ide-bridge`: termherd as an MCP **client** of Claude's IDE
  bridge), still unbuilt.

When adding code, ask: *which crate does this belong in?* If the answer is
"`core` should call this adapter directly," the answer is wrong — add a port
trait in `core::ports` and have the adapter implement it.

## The headless core (where logic lives)

`core::App::apply(Event) -> Vec<Effect>` is the Elm-style heart of the system
(`crates/core/src/app.rs`). It is **pure**: no I/O, no clock, no panic. The
GUI translates user actions into `Event`s and performs the returned
`Effect`s. Everything testable lives behind `apply`.

`Workspace` (`crates/core/src/workspace.rs`) is the tab/split pane tree — pure
data, exhaustively unit-testable. The focus path is a `Vec<Branch>` from the
root; mutators (`open`, `split`, …) return `Option<()>` rather than panicking
when invariants are violated. Follow that pattern: surface broken invariants
as `None`/`Err`, never `unwrap`.

## Concurrency model (when adapters arrive)

One tokio runtime, **actor-per-session**: each session is owned by a task
holding its PTY handle and terminal grid. Other parts of the system talk to
it only via channels. There is no shared `&mut Session`. The GUI thread owns
`core::App` and applies events single-threaded. This is the structural fix for
the `realSessionId` race (Q6 in `docs/PRD.md` §4) — keep it.

## Quality bar — non-negotiable

Each rule below is tied to a Q-row in `docs/PRD.md` §4 (the reason the rewrite
exists). Do not relax them locally.

- **`core` and `claude`**: clippy denies `unwrap_used`, `expect_used`, `panic`.
  Tests may use them (`clippy.toml` allows it in tests). Production paths
  return typed errors (`thiserror`).
- **No global mutable state.** No `static mut`, no `lazy_static`, no
  require-time singletons. Construct dependencies in `main()` and inject.
- **One logging stack:** `tracing`. No `println!` outside tests.
- **`unsafe_code = "deny"`** workspace-wide. The lone sanctioned exception is
  `crates/app/src/macos.rs` (AppKit FFI for the Cmd+Q quit path): a `#![cfg(…)]`
  module with a module-scoped `#![allow(unsafe_code)]` and a `// SAFETY:` note
  on every block. Any further exception needs the same — OS-FFI that can't be
  expressed safely, quarantined in its own `cfg`-gated module — not a relaxation
  scattered through otherwise-safe code.
- **Function length is gated.** `clippy::too_many_lines` (threshold 150 in
  `clippy.toml`) fails CI on over-long functions — a proxy for local
  complexity. A function that exceeds it on purpose (a flat dispatcher / layout
  builder) carries a local `#[allow(clippy::too_many_lines)]` with a rationale,
  never a relaxed global threshold.
- **An invariant expressed twice will drift — extract the predicate.** Two
  call sites deciding "has this settled?" with hand-written conditions is a
  bug waiting on the first edit that touches one of them. It bit the
  `wait_for_status` rung: one site treated a session exit as settling a wait,
  the other only compared against the requested statuses, so a wait placed
  after a crash parked until the caller's timeout. Both now go through one
  `settles()` predicate. A doc-comment asserting the rule is *not* enforcement
  — the comment describing the correct behaviour sat directly above the code
  that broke it.
- **A guard is unreachable and goes, or reachable and gets a test — there is
  no third state.** Defensive arithmetic nobody can trigger is not free: it
  reads as a live case to the next reader, and no test can pin it. Mutation
  testing finds these by construction, because a mutant of dead code changes
  nothing observable and survives. In `image::resample_box` the clamps were
  provably unreachable under its shrink-only precondition and went, while
  `count > 0` — all that stands between a box with no readable pixel and a
  divide by zero — was reachable and earned a truncated-buffer test. Both
  survivors looked like missing assertions and were really design smells.

## Conventions

- Coding standards (Tidy First, CUPID & YAGNI, TDD + Reflect, Clean Code) live
  in [`CODING_STANDARDS.md`](CODING_STANDARDS.md). This file (AGENTS.md) takes
  precedence where they collide.
- Markdown prose: 80-col wrap (tables / code blocks exempt, see
  `.markdownlint-cli2.jsonc`).
- Commit messages: no "Claude" signature (per global user instruction).
- No issue numbers (`#NN`) in code comments, doc-comments, or test names —
  git history already links code to its issue, and an in-code `#NN` rots when
  issues are renumbered or migrated. Cite issues in commit/PR bodies and
  `ROADMAP.md`/PRD prose instead. Full rationale in
  [`CONTRIBUTING.md`](CONTRIBUTING.md).
- A reference code in a comment must be resolvable without external context:
  either name the rule in plain language, or use a code **whose source this
  file records.** The one sanctioned code is **`FRn` = the numbered Functional
  Requirements in [`docs/PRD.md`](docs/PRD.md) (§Functional requirements)** —
  e.g. `FR4` is the embedded-terminal requirement, `FR6` splits. Do not coin
  other bare abbreviations; a lone `FR4` is only readable because of this line.
- Status of every feature is tracked in `ROADMAP.md` (MoSCoW from PRD §5).
  Check the ticked/unticked state there before assuming something is built.

## How we track work

Three layers, each owning one thing — no item lives fully in two places:

- **`ROADMAP.md` (+ `docs/PRD.md`)** — the *what* and *why*: features, MoSCoW
  bucket, shipped history with rationale, and design-first epics not yet scoped
  enough to act on (e.g. `F-i18n`, `F-favorites`). Source of truth for whether
  a feature exists.
- **GitHub issues** — the *unit of work*: actionable, scoped tickets. Each
  carries a native **issue type** (`Feature` / `Bug` / `Task`) and one or more
  **`area:*`** labels; `os:*` and `needs-design` are modifiers on top.
- **[Project board](https://github.com/orgs/Termherd/projects/1)** — canonical
  for **priority and order**, held as sortable single-select fields: `Horizon`
  (Now / Next / Later / Parked / Shipped), `Class`, `Effort`, `Severity` (see
  **Priority scheme** below). Edit these there, visually — not in a file.

### Priority scheme

Priority is **two orthogonal axes**, not one `Pn` number (which conflated
impact, urgency, and cost — the `P0`–`P3` labels were retired 2026-07-26).

- **Class** — the *kind of leverage* a Feature delivers: **⚡ Differentiator**
  (the thesis edge) · **🔑 Enabler** (unblocks other work) · **📐 Table-stakes**
  (expected of any such tool; its absence is a wart) · **✨ Polish**
  (ergonomics) · **🎲 Bet** (uncertain — prototype to learn).
- **Effort** — **S / M / L**.
- **Ordering rule**: within a Horizon, **small Differentiators & Enablers
  first**; a **Bet gets a timeboxed probe, not a full build**.

Bugs are **not a Class** — they restore a contract, they don't add leverage.
A `Bug` carries a **Severity** instead (🔴 Critical / 🟠 Major / 🟡 Minor) and
jumps the queue on severity × blast-radius, off the leverage map. A `Task`
(packaging, tooling, chores) carries neither.

So: **Class / Effort / Severity / Horizon = board fields · Type = the native
issue type · Area = `area:*` labels · everything narrative = `ROADMAP.md`.**

The one rule that keeps it sane: an epic **graduates from `ROADMAP.md` to an
issue only when it's scoped enough to do.** A design-first item lives only in
the roadmap until then; once filed as an issue it appears on the board. Mark
the roadmap entry done when its issues close.

Two corollaries that keep the layers in sync (both contributors work from
issues, so a scoped roadmap item with no issue is invisible):

- **When an epic graduates, link it both ways.** Open the issue *and* add its
  `#number` to the ROADMAP entry. Shipped entries already cite their issues; do
  the same for open ones.
- **Design a backlog epic before filing it.** Run `/feature-torture` on a
  design-first item to reach a verdict (ship / reshape / park / split / kill);
  file issues only for the slices that come out scoped. The report lands in
  `.personal/feature-torture/reports/<F-id>.md`; cite it in the ROADMAP entry.
  Items that stay design-first (e.g. `F-keymap-per-command`) live only in the
  roadmap until their blocking design is resolved.
- **`just board-check` reports the issues the board never classified** — filed,
  then invisible to every view. It checks the board only: the roadmap's MoSCoW
  list has no per-entry horizon, so neither "every issue is cited by an entry"
  (it flags refinements that were never features) nor "every unticked entry
  cites an issue" (it flags the design-first items the rule above *wants* to
  live in the roadmap alone) is checkable. Reconciling the roadmap stays a
  human read; the script's own docstring records why. Run it before a planning
  pass.
