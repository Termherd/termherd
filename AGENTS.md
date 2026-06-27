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

# Markdown is also gated in CI
markdownlint-cli2                  # uses .markdownlint-cli2.jsonc
```

Toolchain is pinned to **Rust 1.95.0 / edition 2024** via `rust-toolchain.toml`
(Q10) — do not bump without updating the pin.

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
- `crates/app` — iced GUI shell. Currently a tracing + single-instance stub;
  M1+ will construct adapters in `main()` and inject them into `core::App`.
- `crates/scan` — filesystem discovery adapter (walks `~/.claude/projects`
  via the `claude` codec; implements `core::ports::ProjectScanner`).
- Remaining adapters land per `docs/ARCHITECTURE.md` §15: `pty` (M2),
  `store` (Should, PRD rev. 4), optional `mcp` (Unsure).

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

## Conventions

- Coding standards (Tidy First, CUPID & YAGNI, TDD + Reflect, Clean Code) live
  in [`CODING_STANDARDS.md`](CODING_STANDARDS.md). This file (AGENTS.md) takes
  precedence where they collide.
- Markdown prose: 80-col wrap (tables / code blocks exempt, see
  `.markdownlint-cli2.jsonc`).
- Commit messages: no "Claude" signature (per global user instruction).
- Status of every feature is tracked in `ROADMAP.md` (MoSCoW from PRD §5).
  Check the ticked/unticked state there before assuming something is built.

## How we track work

Three layers, each owning one thing — no item lives fully in two places:

- **`ROADMAP.md` (+ `docs/PRD.md`)** — the *what* and *why*: features, MoSCoW
  bucket, shipped history with rationale, and design-first epics not yet scoped
  enough to act on (e.g. `F-i18n`, `F-favorites`). Source of truth for whether
  a feature exists.
- **GitHub issues** — the *unit of work*: actionable, scoped tickets. Labelled
  `bug`/`enhancement` and a priority `P0`/`P1`/`P2`.
- **GitHub Project board** — the *what's in flight*: a view over the issues
  (status columns seeded from the `P` labels). It holds no truth of its own.

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
